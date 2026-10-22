// Canonical, self-checking checkpoint for the *whole* registry+ledger state.
// This file is included inside execution.rs to keep state fields private.
use crate::codec::{verify_crc, with_crc, Cursor, MAX_CHECKPOINT_BYTES};
use crate::ledger::SnapshotError as LedgerSnapshotError;
use crate::registry::RegistrySnapshotError;

const PROTOCOL_MAGIC_V1: &[u8; 5] = b"TLPS\x01";
const PROTOCOL_MAGIC_V2: &[u8; 5] = b"TLPS\x02";

#[derive(Debug)]
pub enum ProtocolSnapshotError {
    Invalid(&'static str),
    Registry(RegistrySnapshotError),
    Ledger(LedgerSnapshotError),
}
impl core::fmt::Display for ProtocolSnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ProtocolSnapshotError {}

fn protocol_bad(s: &'static str) -> ProtocolSnapshotError {
    ProtocolSnapshotError::Invalid(s)
}
fn write_embedded(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ProtocolSnapshotError> {
    let n = u32::try_from(bytes.len()).map_err(|_| protocol_bad("embedded snapshot too large"))?;
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}
fn read_embedded<'a>(reader: &mut Cursor<'a>) -> Result<&'a [u8], ProtocolSnapshotError> {
    let n = reader
        .u32()
        .map_err(|_| protocol_bad("embedded size truncated"))? as usize;
    if n > MAX_CHECKPOINT_BYTES {
        return Err(protocol_bad("embedded snapshot too large"));
    }
    reader
        .take(n)
        .map_err(|_| protocol_bad("embedded snapshot truncated"))
}

impl ProtocolState {
    /// CRC guards against accidental corruption. A digest or proof of these
    /// bytes is a separate external requirement before trusting chain state.
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, ProtocolSnapshotError> {
        let mut out = if self.finalized_height.is_some() {
            PROTOCOL_MAGIC_V2.to_vec()
        } else {
            PROTOCOL_MAGIC_V1.to_vec()
        };
        out.extend_from_slice(&self.chain.0);
        write_embedded(
            &mut out,
            &self
                .registry
                .snapshot_bytes()
                .map_err(ProtocolSnapshotError::Registry)?,
        )?;
        write_embedded(
            &mut out,
            &self
                .ledger
                .snapshot_bytes()
                .map_err(ProtocolSnapshotError::Ledger)?,
        )?;
        let count =
            u32::try_from(self.processed.len()).map_err(|_| protocol_bad("too many txids"))?;
        out.extend_from_slice(&count.to_be_bytes());
        for txid in &self.processed {
            out.extend_from_slice(&txid.as_bytes());
        }
        match self.last_height {
            None => out.push(0),
            Some(height) => {
                out.push(1);
                out.extend_from_slice(&height.to_be_bytes());
            }
        }
        if let Some(height) = self.finalized_height {
            if height == 0 || self.last_height.is_some_and(|last| last > height) {
                return Err(protocol_bad("invalid finalized height"));
            }
            out.extend_from_slice(&height.to_be_bytes());
        }
        with_crc(out).map_err(|_| protocol_bad("protocol snapshot too large"))
    }

    pub fn from_snapshot_bytes(
        bytes: &[u8],
        expected_chain: ChainId,
        expected_admin: &AccountId,
    ) -> Result<Self, ProtocolSnapshotError> {
        let data =
            verify_crc(bytes).map_err(|_| protocol_bad("protocol snapshot checksum/size"))?;
        let mut reader = Cursor::new(data);
        let version = match reader
            .fixed::<5>()
            .map_err(|_| protocol_bad("protocol magic truncated"))?
        {
            magic if magic == *PROTOCOL_MAGIC_V1 => 1,
            magic if magic == *PROTOCOL_MAGIC_V2 => 2,
            _ => return Err(protocol_bad("wrong protocol version")),
        };
        let chain = ChainId(
            reader
                .fixed::<32>()
                .map_err(|_| protocol_bad("chain id truncated"))?,
        );
        if chain != expected_chain {
            return Err(protocol_bad("wrong chain"));
        }
        let registry = Registry::from_snapshot_bytes(read_embedded(&mut reader)?, expected_admin)
            .map_err(ProtocolSnapshotError::Registry)?;
        let ledger = Ledger::from_snapshot_bytes(read_embedded(&mut reader)?)
            .map_err(ProtocolSnapshotError::Ledger)?;
        let count = reader
            .u32()
            .map_err(|_| protocol_bad("processed count truncated"))? as usize;
        if count > MAX_CHECKPOINT_BYTES / 32 {
            return Err(protocol_bad("processed count excessive"));
        }
        let mut processed = BTreeSet::new();
        for _ in 0..count {
            let txid = TxId::from_bytes(
                reader
                    .fixed::<32>()
                    .map_err(|_| protocol_bad("processed txid truncated"))?,
            );
            if !processed.insert(txid) {
                return Err(protocol_bad("duplicate global txid"));
            }
        }
        let last_height = match reader
            .u8()
            .map_err(|_| protocol_bad("last height flag truncated"))?
        {
            0 => None,
            1 => Some(
                reader
                    .u64()
                    .map_err(|_| protocol_bad("last height truncated"))?,
            ),
            _ => return Err(protocol_bad("noncanonical height option")),
        };
        let finalized_height = if version == 2 {
            Some(
                reader
                    .u64()
                    .map_err(|_| protocol_bad("finalized height truncated"))?,
            )
        } else {
            None
        };
        reader
            .end()
            .map_err(|_| protocol_bad("trailing protocol bytes"))?;
        if finalized_height == Some(0)
            || finalized_height.is_some_and(|height| last_height.is_some_and(|last| height < last))
        {
            return Err(protocol_bad("inconsistent finalized height"));
        }
        if last_height == Some(0) || (processed.is_empty() != last_height.is_none()) {
            return Err(protocol_bad("inconsistent transaction height"));
        }
        if registry
            .last_height()
            .is_some_and(|h| last_height.is_none_or(|v| h > v))
        {
            return Err(protocol_bad("registry ahead of protocol"));
        }
        // Every global replay marker must belong to exactly one component.
        // This catches cross-component replay and inconsistent snapshots.
        let component_sum = registry
            .processed_count()
            .checked_add(ledger.processed_count())
            .ok_or_else(|| protocol_bad("processed count overflow"))?;
        if component_sum != count
            || processed
                .iter()
                .any(|txid| registry.is_processed(*txid) == ledger.is_processed(*txid))
        {
            return Err(protocol_bad("component/global replay mismatch"));
        }
        let state = Self {
            chain,
            registry,
            ledger,
            processed,
            last_height,
            finalized_height,
        };
        if state.snapshot_bytes()? != bytes {
            return Err(protocol_bad("noncanonical protocol encoding"));
        }
        Ok(state)
    }
}
