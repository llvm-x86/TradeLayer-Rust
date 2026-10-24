// Stage 25D: canonical, versioned FULL spot+clearing snapshot.
// This file is include!d by clearing_sandbox.rs so no public mutable
// escrow or replay-marker state is exposed to other crates.
//
// TLCL v1 is an OFFLINE audit artifact. CRC guards accidental corruption,
// not forged authorization, chain consensus, or an economic proof.
use crate::codec::{verify_crc, with_crc, Cursor, MAX_CHECKPOINT_BYTES};
use crate::spot::SpotSnapshotError;

const CLEARING_STATE_MAGIC_V1: &[u8; 5] = b"TLCL\x01";
const CLEARING_STATE_MAGIC_V2: &[u8; 5] = b"TLCL\x02";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkTerms {
    pub notional_e8: u64,
    pub inverse: bool,
    pub rounding: PnlRounding,
}

#[derive(Debug)]
pub enum ClearingSnapshotError {
    Invalid(&'static str),
    Spot(SpotSnapshotError),
    Clearing(ClearingError),
}

impl core::fmt::Display for ClearingSnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ClearingSnapshotError {}

fn cs_bad(reason: &'static str) -> ClearingSnapshotError {
    ClearingSnapshotError::Invalid(reason)
}

fn cs_u32(out: &mut Vec<u8>, count: usize) -> Result<(), ClearingSnapshotError> {
    out.extend_from_slice(
        &u32::try_from(count)
            .map_err(|_| cs_bad("clearing record count too large"))?
            .to_be_bytes(),
    );
    Ok(())
}

fn cs_read_count(reader: &mut Cursor<'_>, bound: usize) -> Result<usize, ClearingSnapshotError> {
    let count = reader
        .u32()
        .map_err(|_| cs_bad("clearing record count truncated"))? as usize;
    if count > bound {
        return Err(cs_bad("clearing record count excessive"));
    }
    Ok(count)
}

fn cs_put_account(out: &mut Vec<u8>, owner: &AccountId) {
    out.push(owner.as_str().len() as u8);
    out.extend_from_slice(owner.as_str().as_bytes());
}

fn cs_read_account(reader: &mut Cursor<'_>) -> Result<AccountId, ClearingSnapshotError> {
    let size = reader
        .u8()
        .map_err(|_| cs_bad("account length truncated"))? as usize;
    if size == 0 || size > crate::ledger::MAX_ACCOUNT_BYTES {
        return Err(cs_bad("account size invalid"));
    }
    let bytes = reader.take(size).map_err(|_| cs_bad("account truncated"))?;
    let value = core::str::from_utf8(bytes).map_err(|_| cs_bad("account encoding"))?;
    AccountId::try_from(value).map_err(|_| cs_bad("account characters"))
}

fn cs_read_bool(reader: &mut Cursor<'_>) -> Result<bool, ClearingSnapshotError> {
    match reader.u8().map_err(|_| cs_bad("flag truncated"))? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(cs_bad("noncanonical flag")),
    }
}

fn cs_read_rounding(reader: &mut Cursor<'_>) -> Result<PnlRounding, ClearingSnapshotError> {
    match reader.u8().map_err(|_| cs_bad("rounding mode truncated"))? {
        0 => Ok(PnlRounding::Legacy20DecimalPlaces),
        1 => Ok(PnlRounding::ExactRational),
        _ => Err(cs_bad("unknown rounding mode")),
    }
}
fn cs_write_rounding(out: &mut Vec<u8>, value: PnlRounding) {
    out.push(match value {
        PnlRounding::Legacy20DecimalPlaces => 0,
        PnlRounding::ExactRational => 1,
    });
}

impl ClearingSandbox {
    /// TLCL v1: sole SpotExchange Ledger, all spot history, margins,
    /// positions, immutable economic policy and all clearing replay IDs.
    /// This is distinct from historical TLSP/TLJP/TLJC checkpoint lanes.
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, ClearingSnapshotError> {
        self.verify().map_err(ClearingSnapshotError::Clearing)?;
        let mut bytes = if self.insurance_policy.is_some() {
            CLEARING_STATE_MAGIC_V2.to_vec()
        } else {
            CLEARING_STATE_MAGIC_V1.to_vec()
        };
        bytes.push(self.network.tag());
        bytes.extend_from_slice(&self.network.genesis().0);
        bytes.extend_from_slice(&self.network.chain_id().0);
        bytes.extend_from_slice(&self.contract.to_be_bytes());
        bytes.extend_from_slice(&self.collateral.get().to_be_bytes());
        match self.collateral_policy {
            None => bytes.push(0),
            Some(policy) => {
                bytes.push(1);
                bytes.extend_from_slice(&policy.initial_atoms_per_contract.to_be_bytes());
                bytes.extend_from_slice(&policy.maintenance_atoms_per_contract.to_be_bytes());
            }
        }
        match self.mark_policy {
            None => bytes.push(0),
            Some(mark) => {
                bytes.push(1);
                bytes.extend_from_slice(&mark.height.to_be_bytes());
                bytes.extend_from_slice(&mark.price_e8.to_be_bytes());
                bytes.extend_from_slice(&mark.notional_e8.to_be_bytes());
                bytes.push(u8::from(mark.inverse));
                cs_write_rounding(&mut bytes, mark.rounding);
            }
        }
        if let Some(insurance) = &self.insurance_policy {
            cs_put_account(&mut bytes, &insurance.sponsor);
        }
        bytes.push(u8::from(self.spot.has_clearing_activity_unverified()));
        cs_u32(&mut bytes, self.margin_escrows.len())?;
        for (owner, escrow) in &self.margin_escrows {
            cs_put_account(&mut bytes, owner);
            cs_put_account(&mut bytes, escrow);
        }
        cs_u32(&mut bytes, self.positions.len())?;
        for (owner, size) in &self.positions {
            cs_put_account(&mut bytes, owner);
            bytes.extend_from_slice(&size.to_be_bytes());
        }
        cs_u32(&mut bytes, self.seen.len())?;
        for id in &self.seen {
            bytes.extend_from_slice(&id.as_bytes());
        }
        let spot = self
            .spot
            .combined_spot_snapshot_bytes_unverified()
            .map_err(ClearingSnapshotError::Spot)?;
        cs_u32(&mut bytes, spot.len())?;
        bytes.extend_from_slice(&spot);
        with_crc(bytes).map_err(|_| cs_bad("clearing snapshot exceeds size bound"))
    }

    /// Caller-independent policy must be supplied separately from the
    /// bytes: chain identity, contract, collateral, permitted tokens, and
    /// optional immutable collateral/mark terms are NOT trusted just because
    /// a checksummed file contains them.
    pub fn from_snapshot_bytes(
        bytes: &[u8],
        network: TradeLayerMainnet,
        expected_contract: u32,
        expected_collateral: PropertyId,
        expected_permitted: &BTreeSet<PropertyId>,
        expected_collateral_policy: Option<CollateralPolicy>,
        expected_mark_terms: Option<MarkTerms>,
    ) -> Result<Self, ClearingSnapshotError> {
        Self::from_snapshot_bytes_with_insurance(
            bytes,
            network,
            expected_contract,
            expected_collateral,
            expected_permitted,
            expected_collateral_policy,
            expected_mark_terms,
            None,
        )
    }

    /// Only the explicit independent insurer sponsor expected by the caller
    /// allows TLCL v2 restoration. A v1 snapshot can never conjure an
    /// insurer policy or a previously reserved amount.
    #[allow(clippy::too_many_arguments)]
    pub fn from_snapshot_bytes_with_insurance(
        bytes: &[u8],
        network: TradeLayerMainnet,
        expected_contract: u32,
        expected_collateral: PropertyId,
        expected_permitted: &BTreeSet<PropertyId>,
        expected_collateral_policy: Option<CollateralPolicy>,
        expected_mark_terms: Option<MarkTerms>,
        expected_insurance_policy: Option<&InsurancePolicy>,
    ) -> Result<Self, ClearingSnapshotError> {
        let payload = verify_crc(bytes).map_err(|_| cs_bad("clearing CRC or size"))?;
        let mut r = Cursor::new(payload);
        let version = r.fixed::<5>().map_err(|_| cs_bad("clearing magic"))?;
        let with_insurance = match version {
            version if version == *CLEARING_STATE_MAGIC_V1 => false,
            version if version == *CLEARING_STATE_MAGIC_V2 => true,
            _ => return Err(cs_bad("clearing snapshot version")),
        };
        if with_insurance != expected_insurance_policy.is_some() {
            return Err(cs_bad("insurance checkpoint policy/version mismatch"));
        }
        let tag = r.u8().map_err(|_| cs_bad("network tag"))?;
        let genesis = r.fixed::<32>().map_err(|_| cs_bad("network genesis"))?;
        let chain_id = r.fixed::<32>().map_err(|_| cs_bad("network chain"))?;
        let contract = r.u32().map_err(|_| cs_bad("contract truncated"))?;
        let collateral = PropertyId::try_from(r.u32().map_err(|_| cs_bad("collateral truncated"))?)
            .map_err(|_| cs_bad("invalid collateral"))?;
        if network.tag() != tag
            || genesis != network.genesis().0
            || chain_id != network.chain_id().0
            || expected_contract != contract
            || expected_collateral != collateral
            || contract == 0
        {
            return Err(cs_bad("network contract or collateral identity mismatch"));
        }
        let collateral_policy = if cs_read_bool(&mut r)? {
            let p = CollateralPolicy {
                initial_atoms_per_contract: r.u128().map_err(|_| cs_bad("initial policy"))?,
                maintenance_atoms_per_contract: r
                    .u128()
                    .map_err(|_| cs_bad("maintenance policy"))?,
            };
            Some(p.validate().map_err(ClearingSnapshotError::Clearing)?)
        } else {
            None
        };
        if collateral_policy != expected_collateral_policy {
            return Err(cs_bad("collateral policy mismatch"));
        }
        let mark_policy = if cs_read_bool(&mut r)? {
            let mark = MarkPolicy {
                height: r.u64().map_err(|_| cs_bad("mark height"))?,
                price_e8: r.u64().map_err(|_| cs_bad("mark price"))?,
                notional_e8: r.u64().map_err(|_| cs_bad("mark notional"))?,
                inverse: cs_read_bool(&mut r)?,
                rounding: cs_read_rounding(&mut r)?,
            };
            Some(mark.validate().map_err(ClearingSnapshotError::Clearing)?)
        } else {
            None
        };
        if mark_policy.map(|m| MarkTerms {
            notional_e8: m.notional_e8,
            inverse: m.inverse,
            rounding: m.rounding,
        }) != expected_mark_terms
        {
            return Err(cs_bad("mark terms mismatch"));
        }
        let insurance_policy = if with_insurance {
            let policy = InsurancePolicy {
                sponsor: cs_read_account(&mut r)?,
            };
            policy.validate().map_err(ClearingSnapshotError::Clearing)?;
            if expected_insurance_policy != Some(&policy) {
                return Err(cs_bad("insurance sponsor policy mismatch"));
            }
            Some(policy)
        } else {
            None
        };
        let clearing_activity = cs_read_bool(&mut r)?;
        let count = cs_read_count(&mut r, MAX_CLEARING_OWNERS)?;
        let mut margin_escrows = BTreeMap::new();
        let mut prev_owner = None;
        for _ in 0..count {
            let owner = cs_read_account(&mut r)?;
            let escrow = cs_read_account(&mut r)?;
            if prev_owner
                .as_ref()
                .is_some_and(|previous| &owner <= previous)
            {
                return Err(cs_bad("unsorted or repeated margin owners"));
            }
            prev_owner = Some(owner.clone());
            margin_escrows.insert(owner, escrow);
        }
        let count = cs_read_count(&mut r, MAX_CLEARING_OWNERS)?;
        let mut positions = BTreeMap::new();
        let mut prev_owner = None;
        for _ in 0..count {
            let owner = cs_read_account(&mut r)?;
            let size = i64::from_be_bytes(r.fixed::<8>().map_err(|_| cs_bad("position quantity"))?);
            if size == 0
                || prev_owner
                    .as_ref()
                    .is_some_and(|previous| &owner <= previous)
            {
                return Err(cs_bad("zero, duplicate or unsorted position"));
            }
            prev_owner = Some(owner.clone());
            positions.insert(owner, size);
        }
        let count = cs_read_count(&mut r, MAX_CLEARING_INPUTS)?;
        let mut seen = BTreeSet::new();
        let mut prev_id = None;
        for _ in 0..count {
            let id = TxId::from_bytes(
                r.fixed::<32>()
                    .map_err(|_| cs_bad("clearing replay marker"))?,
            );
            if prev_id.is_some_and(|previous| id <= previous) {
                return Err(cs_bad("duplicate or unsorted clearing replay marker"));
            }
            prev_id = Some(id);
            seen.insert(id);
        }
        let spot_len = cs_read_count(&mut r, MAX_CHECKPOINT_BYTES)?;
        if spot_len == 0 {
            return Err(cs_bad("zero-sized inner spot"));
        }
        let spot_data = r
            .take(spot_len)
            .map_err(|_| cs_bad("inner spot truncated"))?;
        r.end()
            .map_err(|_| cs_bad("trailing clearing snapshot bytes"))?;
        let spot = SpotExchange::from_combined_spot_snapshot_bytes_unverified(
            spot_data,
            expected_permitted,
            clearing_activity,
        )
        .map_err(ClearingSnapshotError::Spot)?;
        let recovered = Self {
            network,
            contract,
            collateral,
            spot,
            margin_escrows,
            positions,
            collateral_policy,
            mark_policy,
            insurance_policy,
            seen,
        };
        recovered
            .verify()
            .map_err(ClearingSnapshotError::Clearing)?;
        if recovered.snapshot_bytes()?.as_slice() != bytes {
            return Err(cs_bad("noncanonical clearing snapshot"));
        }
        Ok(recovered)
    }
}
