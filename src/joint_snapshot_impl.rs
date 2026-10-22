// Stage 22 TLJP v1: one registry snapshot and one spot-owned ledger snapshot.
// Existing TLPS/TLUC, TLSP/TLSC and TLNP formats remain unchanged.
// The nested CRCs and outer CRC detect accidents, not forged economic state.
use crate::codec::{verify_crc, with_crc, Cursor, MAX_CHECKPOINT_BYTES};
use crate::registry::RegistrySnapshotError;
use crate::spot::SpotSnapshotError;

const JOINT_STATE_MAGIC: &[u8; 5] = b"TLJP\x01";

#[derive(Debug)]
pub enum JointSnapshotError {
    Invalid(&'static str),
    Registry(RegistrySnapshotError),
    Spot(SpotSnapshotError),
    Joint(JointError),
}
impl core::fmt::Display for JointSnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for JointSnapshotError {}

fn joint_bad(reason: &'static str) -> JointSnapshotError {
    JointSnapshotError::Invalid(reason)
}

fn joint_write_embedded(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), JointSnapshotError> {
    let size = u32::try_from(bytes.len()).map_err(|_| joint_bad("embedded length"))?;
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}
fn joint_read_embedded<'a>(reader: &mut Cursor<'a>) -> Result<&'a [u8], JointSnapshotError> {
    let length = reader
        .u32()
        .map_err(|_| joint_bad("missing embedded size"))? as usize;
    if length == 0 || length > MAX_CHECKPOINT_BYTES {
        return Err(joint_bad("invalid embedded size"));
    }
    reader
        .take(length)
        .map_err(|_| joint_bad("truncated embedded snapshot"))
}
fn joint_write_replay_ids(
    out: &mut Vec<u8>,
    ids: &BTreeSet<TxId>,
) -> Result<(), JointSnapshotError> {
    out.extend_from_slice(
        &u32::try_from(ids.len())
            .map_err(|_| joint_bad("input replay count too large"))?
            .to_be_bytes(),
    );
    for id in ids {
        out.extend_from_slice(&id.as_bytes());
    }
    Ok(())
}
fn joint_read_replay_ids(reader: &mut Cursor<'_>) -> Result<BTreeSet<TxId>, JointSnapshotError> {
    let size = reader
        .u32()
        .map_err(|_| joint_bad("replay count truncated"))? as usize;
    if size > MAX_JOINT_REPLAY_MARKERS {
        return Err(joint_bad("replay count excessive"));
    }
    let mut ids = BTreeSet::new();
    let mut last = None;
    for _ in 0..size {
        let id = TxId::from_bytes(
            reader
                .fixed::<32>()
                .map_err(|_| joint_bad("replay ID truncated"))?,
        );
        if last.is_some_and(|prev| id <= prev) || !ids.insert(id) {
            return Err(joint_bad("unsorted or duplicate input replay ID"));
        }
        last = Some(id);
    }
    Ok(ids)
}

impl JointProtocolState {
    /// Canonical TLJP state with only one mutable token Ledger, inside TLSP.
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, JointSnapshotError> {
        self.verify_components_unverified()
            .map_err(JointSnapshotError::Joint)?;
        let mut out = JOINT_STATE_MAGIC.to_vec();
        out.push(self.network.tag());
        out.extend_from_slice(&self.network.genesis().0);
        joint_write_embedded(
            &mut out,
            &self
                .registry
                .snapshot_bytes()
                .map_err(JointSnapshotError::Registry)?,
        )?;
        joint_write_embedded(
            &mut out,
            &self
                .spot
                .snapshot_bytes()
                .map_err(JointSnapshotError::Spot)?,
        )?;
        joint_write_replay_ids(&mut out, &self.global_input_ids)?;
        joint_write_replay_ids(&mut out, &self.ledger_input_ids)?;
        match self.finalized_height {
            None => out.push(0),
            Some(height) => {
                if height == 0 {
                    return Err(joint_bad("zero finalized block"));
                }
                out.push(1);
                out.extend_from_slice(&height.to_be_bytes());
            }
        }
        with_crc(out).map_err(|_| joint_bad("joint state snapshot too large"))
    }

    /// The expected network, protocol admin and spot token policy must come
    /// from independent deployment configuration, not from these bytes.
    pub fn from_snapshot_bytes(
        bytes: &[u8],
        expected_network: TradeLayerMainnet,
        expected_admin: &AccountId,
        expected_permitted: &BTreeSet<PropertyId>,
    ) -> Result<Self, JointSnapshotError> {
        let raw = verify_crc(bytes).map_err(|_| joint_bad("joint state CRC or length"))?;
        let mut reader = Cursor::new(raw);
        if reader
            .fixed::<5>()
            .map_err(|_| joint_bad("joint state magic"))?
            != *JOINT_STATE_MAGIC
        {
            return Err(joint_bad("joint state version"));
        }
        let network =
            TradeLayerMainnet::from_tag(reader.u8().map_err(|_| joint_bad("network tag"))?)
                .map_err(|_| joint_bad("invalid network tag"))?;
        let genesis = reader
            .fixed::<32>()
            .map_err(|_| joint_bad("genesis bytes"))?;
        if network != expected_network || genesis != expected_network.genesis().0 {
            return Err(joint_bad("wrong network or genesis"));
        }
        let registry =
            Registry::from_snapshot_bytes(joint_read_embedded(&mut reader)?, expected_admin)
                .map_err(JointSnapshotError::Registry)?;
        let spot = SpotExchange::from_snapshot_bytes(
            joint_read_embedded(&mut reader)?,
            expected_permitted,
        )
        .map_err(JointSnapshotError::Spot)?;
        let global_input_ids = joint_read_replay_ids(&mut reader)?;
        let ledger_input_ids = joint_read_replay_ids(&mut reader)?;
        let finalized_height = match reader.u8().map_err(|_| joint_bad("finalized flag"))? {
            0 => None,
            1 => Some(reader.u64().map_err(|_| joint_bad("finalized height"))?),
            _ => return Err(joint_bad("invalid finalized flag")),
        };
        reader
            .end()
            .map_err(|_| joint_bad("joint snapshot trailing bytes"))?;
        let result = Self {
            network,
            registry,
            spot,
            global_input_ids,
            ledger_input_ids,
            finalized_height,
        };
        result
            .verify_components_unverified()
            .map_err(JointSnapshotError::Joint)?;
        if result.snapshot_bytes()?.as_slice() != bytes {
            return Err(joint_bad("noncanonical joint state"));
        }
        Ok(result)
    }
}
