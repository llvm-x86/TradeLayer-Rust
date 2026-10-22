//! Checkpoint support compiled *inside* registry.rs so private state stays private.
//! CRC32 is an accidental-corruption check, never a signature or proof.
use crate::codec::{verify_crc, with_crc, Cursor};

const MAX_ENTRIES: usize = 100_000;
const REGISTRY_MAGIC: &[u8; 5] = b"TLRG\x01";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrySnapshotError {
    Invalid(&'static str),
}

impl core::fmt::Display for RegistrySnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RegistrySnapshotError {}

fn err(field: &'static str) -> RegistrySnapshotError {
    RegistrySnapshotError::Invalid(field)
}
fn put_u32(dst: &mut Vec<u8>, value: usize) -> Result<(), RegistrySnapshotError> {
    let value = u32::try_from(value).map_err(|_| err("entry count overflow"))?;
    dst.extend_from_slice(&value.to_be_bytes());
    Ok(())
}
fn put_text(dst: &mut Vec<u8>, value: &str) -> Result<(), RegistrySnapshotError> {
    let length = u16::try_from(value.len()).map_err(|_| err("text too long"))?;
    dst.extend_from_slice(&length.to_be_bytes());
    dst.extend_from_slice(value.as_bytes());
    Ok(())
}
fn put_account(dst: &mut Vec<u8>, account: &AccountId) -> Result<(), RegistrySnapshotError> {
    put_text(dst, account.as_str())
}
fn put_optional_height(dst: &mut Vec<u8>, value: Option<u64>) {
    match value {
        None => dst.push(0),
        Some(height) => {
            dst.push(1);
            dst.extend_from_slice(&height.to_be_bytes());
        }
    }
}
fn read_count(reader: &mut Cursor<'_>) -> Result<usize, RegistrySnapshotError> {
    let n = reader.u32().map_err(|_| err("entry count truncated"))? as usize;
    if n > MAX_ENTRIES {
        return Err(err("entry count exceeds limit"));
    }
    Ok(n)
}
fn read_text(reader: &mut Cursor<'_>, max: usize, allow_empty: bool) -> Result<String, RegistrySnapshotError> {
    let size = u16::from_be_bytes(reader.fixed::<2>().map_err(|_| err("text length truncated"))?) as usize;
    if size > max || (!allow_empty && size == 0) {
        return Err(err("text length out of bounds"));
    }
    let value = reader.take(size).map_err(|_| err("text truncated"))?;
    if !value.iter().all(|b| (0x20..=0x7e).contains(b)) {
        return Err(err("nonprintable registry text"));
    }
    Ok(core::str::from_utf8(value).map_err(|_| err("utf8 registry text"))?.to_owned())
}
fn read_account(reader: &mut Cursor<'_>) -> Result<AccountId, RegistrySnapshotError> {
    let value = read_text(reader, crate::ledger::MAX_ACCOUNT_BYTES, false)?;
    AccountId::try_from(value.as_str()).map_err(|_| err("invalid account"))
}
fn read_flag(reader: &mut Cursor<'_>) -> Result<bool, RegistrySnapshotError> {
    match reader.u8().map_err(|_| err("flag truncated"))? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(err("noncanonical flag")),
    }
}
fn read_optional_height(reader: &mut Cursor<'_>) -> Result<Option<u64>, RegistrySnapshotError> {
    if read_flag(reader)? {
        Ok(Some(reader.u64().map_err(|_| err("height truncated"))?))
    } else {
        Ok(None)
    }
}
fn read_quote(reader: &mut Cursor<'_>) -> Result<Option<OracleQuote>, RegistrySnapshotError> {
    if !read_flag(reader)? {
        return Ok(None);
    }
    let price_1e4 = reader.u128().map_err(|_| err("price truncated"))?;
    let mut prices = [None; 3];
    for price in &mut prices {
        if read_flag(reader)? {
            *price = Some(reader.u128().map_err(|_| err("optional quote truncated"))?);
        }
    }
    let height = reader.u64().map_err(|_| err("quote height truncated"))?;
    if price_1e4 == 0 || height == 0 || prices.iter().flatten().any(|p| *p == 0) {
        return Err(err("invalid quote"));
    }
    if let (Some(high), Some(low)) = (prices[0], prices[1]) {
        if high < low { return Err(err("inverted quote range")); }
    }
    Ok(Some(OracleQuote { price_1e4, high_1e4: prices[0], low_1e4: prices[1], close_1e4: prices[2], height }))
}
fn put_quote(dst: &mut Vec<u8>, quote: &Option<OracleQuote>) {
    match quote {
        None => dst.push(0),
        Some(q) => {
            dst.push(1);
            dst.extend_from_slice(&q.price_1e4.to_be_bytes());
            for price in [q.high_1e4, q.low_1e4, q.close_1e4] {
                match price {
                    None => dst.push(0),
                    Some(p) => { dst.push(1); dst.extend_from_slice(&p.to_be_bytes()); },
                }
            }
            dst.extend_from_slice(&q.height.to_be_bytes());
        }
    }
}

impl Registry {
    /// The external chain config must supply the protocol admin when restoring.
    pub fn protocol_admin(&self) -> &AccountId {
        &self.protocol_admin
    }

    pub fn is_processed(&self, txid: TxId) -> bool {
        self.processed.contains(&txid)
    }

    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, RegistrySnapshotError> {
        let mut out = REGISTRY_MAGIC.to_vec();
        put_account(&mut out, &self.protocol_admin)?;
        for activation in &self.activations {
            match activation {
                None => out.push(0),
                Some(activation) => {
                    out.push(1);
                    out.extend_from_slice(&activation.height.to_be_bytes());
                    out.extend_from_slice(&activation.code_hash);
                }
            }
        }
        out.extend_from_slice(&self.next_whitelist_id.to_be_bytes());
        out.extend_from_slice(&self.next_oracle_id.to_be_bytes());
        put_u32(&mut out, self.whitelists.len())?;
        for (id, item) in &self.whitelists {
            out.extend_from_slice(&id.to_be_bytes());
            put_account(&mut out, &item.admin)?;
            put_account(&mut out, &item.backup)?;
            put_text(&mut out, &item.name)?;
            put_text(&mut out, &item.url)?;
            put_text(&mut out, &item.description)?;
        }
        put_u32(&mut out, self.attestations.len())?;
        for ((list, account), att) in &self.attestations {
            out.extend_from_slice(&list.to_be_bytes());
            put_account(&mut out, account)?;
            put_text(&mut out, &att.metadata)?;
            out.extend_from_slice(&att.issued_height.to_be_bytes());
            out.extend_from_slice(&att.updated_height.to_be_bytes());
            out.push(u8::from(att.active));
        }
        put_u32(&mut out, self.oracles.len())?;
        for (id, item) in &self.oracles {
            out.extend_from_slice(&id.to_be_bytes());
            put_account(&mut out, &item.admin)?;
            put_account(&mut out, &item.backup)?;
            put_text(&mut out, &item.ticker)?;
            put_text(&mut out, &item.url)?;
            out.extend_from_slice(&item.lag.to_be_bytes());
            out.push(u8::from(item.closed));
            put_quote(&mut out, &item.latest);
        }
        put_optional_height(&mut out, self.last_height);
        put_u32(&mut out, self.processed.len())?;
        for id in &self.processed { out.extend_from_slice(&id.as_bytes()); }
        with_crc(out).map_err(|_| err("registry snapshot too large"))
    }

    /// Restore only from data bound to the independently configured admin.
    /// Full canonical re-serialization rejects duplicate keys and altered encodings.
    pub fn from_snapshot_bytes(bytes: &[u8], expected_admin: &AccountId) -> Result<Self, RegistrySnapshotError> {
        let data = verify_crc(bytes).map_err(|_| err("registry checksum or size"))?;
        let mut r = Cursor::new(data);
        if r.fixed::<5>().map_err(|_| err("registry magic truncated"))? != *REGISTRY_MAGIC {
            return Err(err("wrong registry magic"));
        }
        if &read_account(&mut r)? != expected_admin { return Err(err("registry admin mismatch")); }
        let mut state = Self::new(expected_admin.clone());
        for idx in 0..36 {
            if read_flag(&mut r)? {
                let height = r.u64().map_err(|_| err("activation height truncated"))?;
                let code_hash = r.fixed::<32>().map_err(|_| err("activation hash truncated"))?;
                if height == 0 || idx == 34 { return Err(err("invalid activation")); }
                state.activations[idx] = Some(Activation { height, code_hash });
            }
        }
        state.next_whitelist_id = r.u32().map_err(|_| err("next whitelist id truncated"))?;
        state.next_oracle_id = r.u32().map_err(|_| err("next oracle id truncated"))?;
        for _ in 0..read_count(&mut r)? {
            let id = r.u32().map_err(|_| err("whitelist id truncated"))?;
            let item = Whitelist {
                admin: read_account(&mut r)?, backup: read_account(&mut r)?,
                name: read_text(&mut r, MAX_NAME_BYTES, false)?,
                url: read_text(&mut r, MAX_URL_BYTES, true)?,
                description: read_text(&mut r, MAX_METADATA_BYTES, true)?,
            };
            if id == 0 || state.whitelists.insert(id, item).is_some() { return Err(err("invalid whitelist id")); }
        }
        for _ in 0..read_count(&mut r)? {
            let list_id = r.u32().map_err(|_| err("attestation list truncated"))?;
            let subject = read_account(&mut r)?;
            let metadata = read_text(&mut r, MAX_METADATA_BYTES, true)?;
            let issued_height = r.u64().map_err(|_| err("attestation issue height truncated"))?;
            let updated_height = r.u64().map_err(|_| err("attestation update height truncated"))?;
            let active = read_flag(&mut r)?;
            if !state.whitelists.contains_key(&list_id) || issued_height == 0 || updated_height < issued_height {
                return Err(err("invalid attestation"));
            }
            if state.attestations.insert((list_id, subject), Attestation { metadata, issued_height, updated_height, active }).is_some() {
                return Err(err("duplicate attestation"));
            }
        }
        for _ in 0..read_count(&mut r)? {
            let id = r.u32().map_err(|_| err("oracle id truncated"))?;
            let item = Oracle {
                admin: read_account(&mut r)?, backup: read_account(&mut r)?,
                ticker: read_text(&mut r, MAX_NAME_BYTES, false)?,
                url: read_text(&mut r, MAX_URL_BYTES, true)?,
                lag: r.u32().map_err(|_| err("oracle lag truncated"))?,
                closed: read_flag(&mut r)?,
                latest: read_quote(&mut r)?,
            };
            if id == 0 || item.lag == 0 || state.oracles.insert(id, item).is_some() {
                return Err(err("invalid oracle"));
            }
        }
        state.last_height = read_optional_height(&mut r)?;
        for _ in 0..read_count(&mut r)? {
            let txid = TxId::from_bytes(r.fixed::<32>().map_err(|_| err("processed txid truncated"))?);
            if !state.processed.insert(txid) { return Err(err("duplicate processed id")); }
        }
        r.end().map_err(|_| err("trailing registry data"))?;
        let max_whitelist = state.whitelists.keys().next_back().copied().unwrap_or(0);
        let max_oracle = state.oracles.keys().next_back().copied().unwrap_or(0);
        if state.next_whitelist_id <= max_whitelist || state.next_oracle_id <= max_oracle
            || state.next_whitelist_id == 0 || state.next_oracle_id == 0 {
            return Err(err("noncanonical next id"));
        }
        let latest = state.last_height.unwrap_or(0);
        if (state.processed_count() > 0 && latest == 0)
            || state.activations.iter().flatten().any(|a| a.height > latest)
            || state.attestations.values().any(|a| a.updated_height > latest)
            || state.oracles.values().filter_map(|o| o.latest.as_ref()).any(|q| q.height > latest) {
            return Err(err("registry history extends beyond tip"));
        }
        if state.snapshot_bytes()? != bytes { return Err(err("noncanonical registry encoding")); }
        Ok(state)
    }
}
