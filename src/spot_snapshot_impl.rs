// Canonical audit-only spot+ledger snapshot. Included by spot.rs to keep
// book, replay markers and escrow ownership private. CRC guards accidental
// corruption, not maliciously constructed checkpoint files.
use crate::codec::{verify_crc, with_crc, Cursor, MAX_CHECKPOINT_BYTES};
use crate::ledger::SnapshotError as LedgerSnapshotError;

const SPOT_MAGIC: &[u8; 5] = b"TLSP\x01";

#[derive(Debug)]
pub enum SpotSnapshotError {
    Invalid(&'static str),
    Ledger(LedgerSnapshotError),
    Spot(SpotError),
}
impl core::fmt::Display for SpotSnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SpotSnapshotError {}

fn spot_invalid(reason: &'static str) -> SpotSnapshotError {
    SpotSnapshotError::Invalid(reason)
}
fn spot_u32(out: &mut Vec<u8>, count: usize) -> Result<(), SpotSnapshotError> {
    out.extend_from_slice(
        &u32::try_from(count)
            .map_err(|_| spot_invalid("spot count too large"))?
            .to_be_bytes(),
    );
    Ok(())
}
fn spot_length(reader: &mut Cursor<'_>, limit: usize) -> Result<usize, SpotSnapshotError> {
    let size = reader
        .u32()
        .map_err(|_| spot_invalid("truncated spot count"))? as usize;
    if size > limit {
        return Err(spot_invalid("spot count exceeds policy"));
    }
    Ok(size)
}
fn spot_put_account(out: &mut Vec<u8>, account: &AccountId) {
    out.push(account.as_str().len() as u8);
    out.extend_from_slice(account.as_str().as_bytes());
}
fn spot_read_account(reader: &mut Cursor<'_>) -> Result<AccountId, SpotSnapshotError> {
    let len = reader.u8().map_err(|_| spot_invalid("account length"))? as usize;
    if len == 0 || len > crate::ledger::MAX_ACCOUNT_BYTES {
        return Err(spot_invalid("account size"));
    }
    let raw = reader
        .take(len)
        .map_err(|_| spot_invalid("account bytes"))?;
    let text = core::str::from_utf8(raw).map_err(|_| spot_invalid("account utf8"))?;
    AccountId::try_from(text).map_err(|_| spot_invalid("account characters"))
}
fn spot_read_property(reader: &mut Cursor<'_>) -> Result<PropertyId, SpotSnapshotError> {
    let id = reader
        .u32()
        .map_err(|_| spot_invalid("property truncated"))?;
    PropertyId::try_from(id).map_err(|_| spot_invalid("invalid property id"))
}
fn spot_read_txid(reader: &mut Cursor<'_>) -> Result<TxId, SpotSnapshotError> {
    Ok(TxId::from_bytes(
        reader
            .fixed::<32>()
            .map_err(|_| spot_invalid("txid truncated"))?,
    ))
}
fn spot_put_pair(out: &mut Vec<u8>, pair: SpotPair) {
    out.extend_from_slice(&pair.base.get().to_be_bytes());
    out.extend_from_slice(&pair.quote.get().to_be_bytes());
}
fn spot_read_pair(reader: &mut Cursor<'_>) -> Result<SpotPair, SpotSnapshotError> {
    let base = spot_read_property(reader)?;
    let quote = spot_read_property(reader)?;
    let pair = SpotPair::new(base, quote).map_err(SpotSnapshotError::Spot)?;
    if pair.base != base {
        return Err(spot_invalid("noncanonical pair ordering"));
    }
    Ok(pair)
}

impl SpotExchange {
    /// Re-run independently checkable invariants for each serialized state
    /// before encoding or publishing a recovered checkpoint.
    fn validate_spot_snapshot_mode(
        &self,
        allow_combined_clearing: bool,
    ) -> Result<(), SpotSnapshotError> {
        if self.clearing_activity && !allow_combined_clearing {
            return Err(spot_invalid(
                "clearing event history requires a combined checkpoint",
            ));
        }
        // An isolated TLSP snapshot cannot restore clearing positions or
        // escrow ownership metadata. Refuse all non-spot reserved balances
        // rather than accepting a stranded spendable collateral account.
        if !allow_combined_clearing
            && self
                .ledger
                .balances_with_account_prefix("~")
                .iter()
                .any(|(_, account, _)| !account.as_str().starts_with(ESCROW_PREFIX))
        {
            return Err(spot_invalid("non-spot escrow needs combined checkpoint"));
        }
        // An inner TLSP payload may be carried by the new combined clearing
        // format, but a public TLSP writer/reader NEVER accepts it directly.
        // Only the outer clearing decoder can validate and own the associated
        // margin and replay metadata.
        if self
            .permitted
            .iter()
            .any(|p| p.get() <= 4 || self.ledger.property(*p).is_none())
        {
            return Err(spot_invalid("invalid permitted token policy"));
        }
        if self.seen.len() > MAX_SPOT_INPUT_EVENTS
            || self.fills.len() > MAX_SPOT_FILLS
            || self.queued.len().saturating_add(self.open_count()) > MAX_SPOT_OPEN_AND_QUEUED
        {
            return Err(spot_invalid("spot capacity"));
        }
        if self.processed_through == Some(0) {
            return Err(spot_invalid("zero block boundary"));
        }
        let height = self.processed_through.unwrap_or(0);
        let mut active_ids = BTreeSet::new();
        let mut sequences = BTreeSet::new();
        for queued in &self.queued {
            if queued.height <= height
                || queued.actor.as_str().starts_with('~')
                || queued.intent.post
                || queued.intent.stop
                || queued.intent.offered_atoms == 0
                || queued.intent.expected_atoms == 0
            {
                return Err(spot_invalid("invalid queued order"));
            }
            self.check_pair(queued.intent.offered, queued.intent.desired)
                .map_err(SpotSnapshotError::Spot)?;
            if !self.seen.contains(&queued.txid) || !active_ids.insert(queued.txid) {
                return Err(spot_invalid("queued replay mismatch"));
            }
        }
        if self.processed_through.is_none() {
            if !self.books.is_empty()
                || !self.fills.is_empty()
                || self.next_sequence != 0
                || active_ids.len() != self.seen.len()
            {
                return Err(spot_invalid("nonempty unprocessed book or replay"));
            }
            self.ledger
                .verify_conservation()
                .map_err(SpotError::Ledger)
                .map_err(SpotSnapshotError::Spot)?;
            self.verify_reserves().map_err(SpotSnapshotError::Spot)?;
            return Ok(());
        }
        for (pair, orders) in &self.books {
            if *pair != SpotPair::new(pair.base, pair.quote).map_err(SpotSnapshotError::Spot)? {
                return Err(spot_invalid("noncanonical book"));
            }
            self.check_pair(pair.base, pair.quote)
                .map_err(SpotSnapshotError::Spot)?;
            let mut previous_sequence = None;
            for order in orders {
                if SpotPair::new(order.offered, order.desired) != Ok(*pair)
                    || order.height == 0
                    || order.height > height
                    || order.owner.as_str().starts_with('~')
                    || order.offered_atoms == 0
                    || order.expected_atoms == 0
                    || order.remaining_offered == 0
                    || order.remaining_offered > order.offered_atoms
                    || order.remaining_desired == 0
                    || order.remaining_desired > order.expected_atoms
                    || order.sequence >= self.next_sequence
                    || previous_sequence.is_some_and(|old| old >= order.sequence)
                    || !self.seen.contains(&order.txid)
                    || !self
                        .ledger
                        .is_processed(internal_txid(order.txid, b"reserve", 0))
                    || !active_ids.insert(order.txid)
                    || !sequences.insert(order.sequence)
                {
                    return Err(spot_invalid("invalid resting order"));
                }
                previous_sequence = Some(order.sequence);
            }
        }
        for (index, fill) in self.fills.iter().enumerate() {
            let nonce = u32::try_from(index).map_err(|_| spot_invalid("fill index"))?;
            if fill.at_height == 0
                || fill.at_height > height
                || fill.base_atoms == 0
                || fill.quote_atoms == 0
                || !self.seen.contains(&fill.sell_txid)
                || !self.seen.contains(&fill.buy_txid)
                || !self.seen.contains(&fill.trigger_txid)
                || !self
                    .ledger
                    .is_processed(internal_txid(fill.trigger_txid, b"fill-base", nonce))
                || !self
                    .ledger
                    .is_processed(internal_txid(fill.trigger_txid, b"fill-quote", nonce))
            {
                return Err(spot_invalid("invalid fill history"));
            }
            self.check_pair(fill.pair.base, fill.pair.quote)
                .map_err(SpotSnapshotError::Spot)?;
        }
        self.ledger
            .verify_conservation()
            .map_err(SpotError::Ledger)
            .map_err(SpotSnapshotError::Spot)?;
        self.verify_reserves().map_err(SpotSnapshotError::Spot)?;
        Ok(())
    }

    /// One canonical versioned snapshot containing all *currently observable*
    /// order book information, replay IDs and the complete associated ledger.
    /// It is an offline research artifact, not an authenticated state root.
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, SpotSnapshotError> {
        self.snapshot_bytes_mode(false)
    }

    /// Only the versioned clearing checkpoint may call this; older TLSP,
    /// TLSC, TLJP and TLJC remain fail-closed after any clearing activity.
    pub(crate) fn combined_spot_snapshot_bytes_unverified(
        &self,
    ) -> Result<Vec<u8>, SpotSnapshotError> {
        self.snapshot_bytes_mode(true)
    }

    fn snapshot_bytes_mode(
        &self,
        allow_combined_clearing: bool,
    ) -> Result<Vec<u8>, SpotSnapshotError> {
        self.validate_spot_snapshot_mode(allow_combined_clearing)?;
        let mut out = SPOT_MAGIC.to_vec();
        spot_u32(&mut out, self.permitted.len())?;
        for p in &self.permitted {
            out.extend_from_slice(&p.get().to_be_bytes());
        }
        let ledger = self
            .ledger
            .snapshot_bytes()
            .map_err(SpotSnapshotError::Ledger)?;
        spot_u32(&mut out, ledger.len())?;
        out.extend_from_slice(&ledger);
        spot_u32(&mut out, self.queued.len())?;
        for entry in &self.queued {
            out.extend_from_slice(&entry.txid.as_bytes());
            spot_put_account(&mut out, &entry.actor);
            out.extend_from_slice(&entry.height.to_be_bytes());
            out.extend_from_slice(&entry.intent.offered.get().to_be_bytes());
            out.extend_from_slice(&entry.intent.desired.get().to_be_bytes());
            out.extend_from_slice(&entry.intent.offered_atoms.to_be_bytes());
            out.extend_from_slice(&entry.intent.expected_atoms.to_be_bytes());
            out.push(u8::from(entry.intent.post));
            out.push(u8::from(entry.intent.stop));
        }
        spot_u32(&mut out, self.books.len())?;
        for (pair, orders) in &self.books {
            spot_put_pair(&mut out, *pair);
            spot_u32(&mut out, orders.len())?;
            for order in orders {
                out.extend_from_slice(&order.txid.as_bytes());
                spot_put_account(&mut out, &order.owner);
                out.extend_from_slice(&order.height.to_be_bytes());
                out.extend_from_slice(&order.offered.get().to_be_bytes());
                out.extend_from_slice(&order.desired.get().to_be_bytes());
                out.extend_from_slice(&order.offered_atoms.to_be_bytes());
                out.extend_from_slice(&order.expected_atoms.to_be_bytes());
                out.extend_from_slice(&order.remaining_offered.to_be_bytes());
                out.extend_from_slice(&order.remaining_desired.to_be_bytes());
                out.extend_from_slice(&order.sequence.to_be_bytes());
            }
        }
        spot_u32(&mut out, self.seen.len())?;
        for id in &self.seen {
            out.extend_from_slice(&id.as_bytes());
        }
        spot_u32(&mut out, self.fills.len())?;
        for fill in &self.fills {
            out.extend_from_slice(&fill.sell_txid.as_bytes());
            out.extend_from_slice(&fill.buy_txid.as_bytes());
            spot_put_pair(&mut out, fill.pair);
            out.extend_from_slice(&fill.base_atoms.to_be_bytes());
            out.extend_from_slice(&fill.quote_atoms.to_be_bytes());
            out.extend_from_slice(&fill.trigger_txid.as_bytes());
            out.extend_from_slice(&fill.at_height.to_be_bytes());
        }
        out.extend_from_slice(&self.next_sequence.to_be_bytes());
        match self.processed_through {
            None => out.push(0),
            Some(height) => {
                out.push(1);
                out.extend_from_slice(&height.to_be_bytes());
            }
        }
        with_crc(out).map_err(|_| spot_invalid("spot snapshot too large"))
    }

    /// A read requires the permitted asset set from independent caller policy.
    /// Checksummed input can still be maliciously forged, so keep this
    /// entire lane outside trusted chain and settlement adapters.
    pub fn from_snapshot_bytes(
        bytes: &[u8],
        expected_permitted: &BTreeSet<PropertyId>,
    ) -> Result<Self, SpotSnapshotError> {
        Self::from_snapshot_bytes_mode(bytes, expected_permitted, false, false)
    }

    /// Deliberately inaccessible outside this crate: restoring an inner TLSP
    /// with margin balances requires the complete surrounding clearing state.
    pub(crate) fn from_combined_spot_snapshot_bytes_unverified(
        bytes: &[u8],
        expected_permitted: &BTreeSet<PropertyId>,
        clearing_activity: bool,
    ) -> Result<Self, SpotSnapshotError> {
        Self::from_snapshot_bytes_mode(bytes, expected_permitted, true, clearing_activity)
    }

    fn from_snapshot_bytes_mode(
        bytes: &[u8],
        expected_permitted: &BTreeSet<PropertyId>,
        allow_combined_clearing: bool,
        clearing_activity: bool,
    ) -> Result<Self, SpotSnapshotError> {
        let payload = verify_crc(bytes).map_err(|_| spot_invalid("spot CRC or size"))?;
        let mut reader = Cursor::new(payload);
        if reader
            .fixed::<5>()
            .map_err(|_| spot_invalid("spot magic"))?
            != *SPOT_MAGIC
        {
            return Err(spot_invalid("spot magic/version"));
        }
        let count = spot_length(&mut reader, MAX_SPOT_INPUT_EVENTS)?;
        let mut permitted = BTreeSet::new();
        let mut last = None;
        for _ in 0..count {
            let id = spot_read_property(&mut reader)?;
            if last.is_some_and(|p| id <= p) {
                return Err(spot_invalid("duplicate or unsorted policy"));
            }
            permitted.insert(id);
            last = Some(id);
        }
        if &permitted != expected_permitted {
            return Err(spot_invalid("permitted token policy mismatch"));
        }
        let ledger_size = spot_length(&mut reader, MAX_CHECKPOINT_BYTES)?;
        let ledger = Ledger::from_snapshot_bytes(
            reader
                .take(ledger_size)
                .map_err(|_| spot_invalid("ledger truncated"))?,
        )
        .map_err(SpotSnapshotError::Ledger)?;
        let queued_count = spot_length(&mut reader, MAX_SPOT_OPEN_AND_QUEUED)?;
        let mut queued = Vec::with_capacity(queued_count);
        for _ in 0..queued_count {
            let txid = spot_read_txid(&mut reader)?;
            let actor = spot_read_account(&mut reader)?;
            let height = reader.u64().map_err(|_| spot_invalid("queued height"))?;
            let offered = spot_read_property(&mut reader)?;
            let desired = spot_read_property(&mut reader)?;
            let offered_atoms = reader.u64().map_err(|_| spot_invalid("queued amount"))?;
            let expected_atoms = reader.u64().map_err(|_| spot_invalid("queued wanted"))?;
            let post = reader.u8().map_err(|_| spot_invalid("post flag"))?;
            let stop = reader.u8().map_err(|_| spot_invalid("stop flag"))?;
            if post > 1 || stop > 1 {
                return Err(spot_invalid("noncanonical flags"));
            }
            queued.push(UnverifiedSpotPlacement {
                txid,
                actor,
                height,
                intent: SpotIntent {
                    offered,
                    desired,
                    offered_atoms,
                    expected_atoms,
                    post: post == 1,
                    stop: stop == 1,
                },
            });
        }
        let book_count = spot_length(&mut reader, MAX_SPOT_OPEN_AND_QUEUED)?;
        let mut books = BTreeMap::new();
        let mut last_pair = None;
        for _ in 0..book_count {
            let pair = spot_read_pair(&mut reader)?;
            if last_pair.is_some_and(|old| pair <= old) {
                return Err(spot_invalid("noncanonical book ordering"));
            }
            last_pair = Some(pair);
            let count = spot_length(&mut reader, MAX_SPOT_OPEN_AND_QUEUED)?;
            let mut orders = Vec::with_capacity(count);
            for _ in 0..count {
                orders.push(RestingSpotOrder {
                    txid: spot_read_txid(&mut reader)?,
                    owner: spot_read_account(&mut reader)?,
                    height: reader.u64().map_err(|_| spot_invalid("order height"))?,
                    offered: spot_read_property(&mut reader)?,
                    desired: spot_read_property(&mut reader)?,
                    offered_atoms: reader.u64().map_err(|_| spot_invalid("order offered"))?,
                    expected_atoms: reader.u64().map_err(|_| spot_invalid("order wanted"))?,
                    remaining_offered: reader.u64().map_err(|_| spot_invalid("order reserve"))?,
                    remaining_desired: reader.u64().map_err(|_| spot_invalid("order remaining"))?,
                    sequence: reader.u64().map_err(|_| spot_invalid("order sequence"))?,
                });
            }
            books.insert(pair, orders);
        }
        let seen_count = spot_length(&mut reader, MAX_SPOT_INPUT_EVENTS)?;
        let mut seen = BTreeSet::new();
        let mut last = None;
        for _ in 0..seen_count {
            let txid = spot_read_txid(&mut reader)?;
            if last.is_some_and(|old| txid <= old) {
                return Err(spot_invalid("noncanonical replay markers"));
            }
            last = Some(txid);
            seen.insert(txid);
        }
        let fill_count = spot_length(&mut reader, MAX_SPOT_FILLS)?;
        let mut fills = Vec::with_capacity(fill_count);
        for _ in 0..fill_count {
            fills.push(SpotFill {
                sell_txid: spot_read_txid(&mut reader)?,
                buy_txid: spot_read_txid(&mut reader)?,
                pair: spot_read_pair(&mut reader)?,
                base_atoms: reader.u64().map_err(|_| spot_invalid("fill base"))?,
                quote_atoms: reader.u64().map_err(|_| spot_invalid("fill quote"))?,
                trigger_txid: spot_read_txid(&mut reader)?,
                at_height: reader.u64().map_err(|_| spot_invalid("fill height"))?,
            });
        }
        let next_sequence = reader.u64().map_err(|_| spot_invalid("next sequence"))?;
        let processed_through = match reader.u8().map_err(|_| spot_invalid("height flag"))? {
            0 => None,
            1 => Some(reader.u64().map_err(|_| spot_invalid("processed height"))?),
            _ => return Err(spot_invalid("noncanonical height flag")),
        };
        reader
            .end()
            .map_err(|_| spot_invalid("trailing spot data"))?;
        let result = Self {
            ledger,
            permitted,
            queued,
            books,
            seen,
            fills,
            next_sequence,
            processed_through,
            clearing_activity,
        };
        result.validate_spot_snapshot_mode(allow_combined_clearing)?;
        if result
            .snapshot_bytes_mode(allow_combined_clearing)?
            .as_slice()
            != bytes
        {
            return Err(spot_invalid("noncanonical spot encoding"));
        }
        Ok(result)
    }
}
