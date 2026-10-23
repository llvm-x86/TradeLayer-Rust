// Stage 12: an offline, consensus-UNVERIFIED journal for Type 23 NET_SETTLE
// claims and Type 31 KING_SETTLE channel debits. Included by ledger.rs.
// A SHA-256 root authenticates none of the claimed trades or settlement inputs.
// Real chain and Cairo/STWO proof validation remain external requirements.

use crate::hash256::sha256;

pub const MAX_SETTLEMENT_RECORDS: usize = 10_000;
const SETTLEMENT_ROOT_DOMAIN: &[u8] = b"TL_NET_SETTLEMENT_ROOT_V1";

#[derive(Debug, Clone, PartialEq, Eq)]
struct NetClaim {
    channel: AccountId,
    property: PropertyId,
    payer_side: ChannelSide,
    amount_atoms: u128,
    trade_reference: TxId,
    height: u64,
    expiry_height: u64,
}

/// Deterministic preview of the currently pending claims selected for
/// a scoped king settlement. The root is bookkeeping only, not a ZK proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementPreview {
    pub root: [u8; 32],
    pub count: u32,
    pub payer_side: ChannelSide,
    pub net_atoms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KingReceipt {
    channel: AccountId,
    property: PropertyId,
    start_height: u64,
    end_height: u64,
    settled_at: u64,
    preview: SettlementPreview,
    claim_txids: Vec<TxId>,
}

fn nonzero_side_claim(
    book: &ChannelState,
    channel: &AccountId,
    property: PropertyId,
    side: ChannelSide,
    amount: u128,
) -> Result<(), LedgerError> {
    if book.owner(ChannelSide::A).is_none() || book.owner(ChannelSide::B).is_none() {
        return Err(LedgerError::ChannelNotBilateral(channel.clone()));
    }
    if amount == 0 {
        return Err(LedgerError::ZeroAmount);
    }
    let balance = book.atoms(side, property);
    if balance < amount {
        return Err(LedgerError::InsufficientAvailable {
            property,
            available: balance,
            requested: amount,
        });
    }
    Ok(())
}

/// Build a strictly byte-delimited root. Claims are sorted by txid because
/// all underlying journals use BTreeMap. The channel, property and range are
/// part of the root so the same claim set cannot be relabelled for a new sweep.
fn calculate_settlement_preview(
    channel: &AccountId,
    property: PropertyId,
    start: u64,
    end: u64,
    settled_at: u64,
    claims: &[(TxId, &NetClaim)],
) -> Result<SettlementPreview, LedgerError> {
    if start == 0 || end < start || end > settled_at || claims.is_empty() {
        return Err(LedgerError::InvalidSettlementRange);
    }
    if claims.len() > MAX_SETTLEMENT_RECORDS {
        return Err(LedgerError::SettlementJournalFull);
    }
    let mut hash_input = SETTLEMENT_ROOT_DOMAIN.to_vec();
    let raw_channel = channel.as_str().as_bytes();
    hash_input.push(raw_channel.len() as u8);
    hash_input.extend_from_slice(raw_channel);
    hash_input.extend_from_slice(&property.get().to_be_bytes());
    hash_input.extend_from_slice(&start.to_be_bytes());
    hash_input.extend_from_slice(&end.to_be_bytes());
    hash_input.extend_from_slice(&(claims.len() as u32).to_be_bytes());
    let mut prev = None;
    let (mut a_to_b, mut b_to_a) = (0u128, 0u128);
    for (txid, claim) in claims {
        if prev.is_some_and(|last| *txid <= last) {
            return Err(LedgerError::MalformedSettlementJournal);
        }
        prev = Some(*txid);
        if &claim.channel != channel
            || claim.property != property
            || !(start..=end).contains(&claim.height)
            || claim.expiry_height < settled_at
            || claim.amount_atoms == 0
        {
            return Err(LedgerError::InvalidSettlementClaim);
        }
        match claim.payer_side {
            ChannelSide::A => {
                a_to_b = a_to_b
                    .checked_add(claim.amount_atoms)
                    .ok_or(LedgerError::ArithmeticOverflow)?;
            }
            ChannelSide::B => {
                b_to_a = b_to_a
                    .checked_add(claim.amount_atoms)
                    .ok_or(LedgerError::ArithmeticOverflow)?;
            }
        }
        hash_input.extend_from_slice(&txid.as_bytes());
        hash_input.push(claim.payer_side.tag());
        hash_input.extend_from_slice(&claim.amount_atoms.to_be_bytes());
        hash_input.extend_from_slice(&claim.trade_reference.as_bytes());
        hash_input.extend_from_slice(&claim.height.to_be_bytes());
        hash_input.extend_from_slice(&claim.expiry_height.to_be_bytes());
    }
    if a_to_b == b_to_a {
        return Err(LedgerError::ZeroNetSettlement);
    }
    let (payer_side, net_atoms) = if a_to_b > b_to_a {
        (ChannelSide::A, a_to_b - b_to_a)
    } else {
        (ChannelSide::B, b_to_a - a_to_b)
    };
    Ok(SettlementPreview {
        root: sha256(&hash_input),
        count: claims.len() as u32,
        payer_side,
        net_atoms,
    })
}

impl Ledger {
    pub fn pending_net_settlement_count(&self) -> usize {
        self.pending_net_settlements.len()
    }
    pub fn swept_net_settlement_count(&self) -> usize {
        self.swept_net_settlements.len()
    }
    pub fn king_settlement_count(&self) -> usize {
        self.king_settlements.len()
    }
    pub fn has_pending_trade_reference(&self, channel: &AccountId, reference: TxId) -> bool {
        self.pending_net_settlements
            .values()
            .any(|claim| &claim.channel == channel && claim.trade_reference == reference)
    }

    fn record_net_settlement(&mut self, txid: TxId, claim: NetClaim) -> Result<(), LedgerError> {
        self.existing_property(claim.property)?;
        if claim.height == 0
            || claim.expiry_height < claim.height
            || claim.trade_reference == txid
            || claim.trade_reference == TxId::from_bytes([0; 32])
        {
            return Err(LedgerError::InvalidSettlementClaim);
        }
        if self.pending_net_settlements.len() + self.swept_net_settlements.len()
            >= MAX_SETTLEMENT_RECORDS
        {
            return Err(LedgerError::SettlementJournalFull);
        }
        if self
            .pending_net_settlements
            .values()
            .chain(self.swept_net_settlements.values())
            .any(|past| {
                past.channel == claim.channel && past.trade_reference == claim.trade_reference
            })
        {
            return Err(LedgerError::DuplicateSettlementReference);
        }
        let book = self
            .channels
            .get(&claim.channel)
            .ok_or_else(|| LedgerError::UnknownChannel(claim.channel.clone()))?;
        nonzero_side_claim(
            book,
            &claim.channel,
            claim.property,
            claim.payer_side,
            claim.amount_atoms,
        )?;
        self.pending_net_settlements.insert(txid, claim);
        Ok(())
    }

    /// No state change. The caller may compute this preview to construct an
    /// offline test fixture, never as proof that the claimed trades happened.
    pub fn preview_king_settlement_unverified(
        &self,
        channel: &AccountId,
        property: PropertyId,
        start_height: u64,
        end_height: u64,
        settled_at: u64,
    ) -> Result<SettlementPreview, LedgerError> {
        let matching: Vec<_> = self
            .pending_net_settlements
            .iter()
            .filter(|(_, claim)| {
                &claim.channel == channel
                    && claim.property == property
                    && (start_height..=end_height).contains(&claim.height)
            })
            .map(|(txid, claim)| (*txid, claim))
            .collect();
        calculate_settlement_preview(
            channel,
            property,
            start_height,
            end_height,
            settled_at,
            &matching,
        )
    }

    fn perform_king_settlement(
        &mut self,
        txid: TxId,
        receipt: KingReceipt,
    ) -> Result<(), LedgerError> {
        if self.king_settlements.len() >= MAX_SETTLEMENT_RECORDS {
            return Err(LedgerError::SettlementJournalFull);
        }
        let selected: Vec<(TxId, NetClaim)> = self
            .pending_net_settlements
            .iter()
            .filter(|(_, claim)| {
                claim.channel == receipt.channel
                    && claim.property == receipt.property
                    && (receipt.start_height..=receipt.end_height).contains(&claim.height)
            })
            .map(|(id, claim)| (*id, claim.clone()))
            .collect();
        let borrowed: Vec<_> = selected.iter().map(|(id, claim)| (*id, claim)).collect();
        let recomputed = calculate_settlement_preview(
            &receipt.channel,
            receipt.property,
            receipt.start_height,
            receipt.end_height,
            receipt.settled_at,
            &borrowed,
        )?;
        if recomputed != receipt.preview
            || selected.iter().map(|(id, _)| *id).collect::<Vec<_>>() != receipt.claim_txids
        {
            return Err(LedgerError::SettlementCommitmentMismatch);
        }
        let state = self
            .channels
            .get_mut(&receipt.channel)
            .ok_or_else(|| LedgerError::UnknownChannel(receipt.channel.clone()))?;
        nonzero_side_claim(
            state,
            &receipt.channel,
            receipt.property,
            recomputed.payer_side,
            recomputed.net_atoms,
        )?;
        let receiving_side = recomputed.payer_side.other();
        let (payer, receiver) = checked_move_for_property(
            receipt.property,
            state.atoms(recomputed.payer_side, receipt.property),
            state.atoms(receiving_side, receipt.property),
            recomputed.net_atoms,
        )?;
        state.put(receiving_side, receipt.property, receiver);
        state.put(recomputed.payer_side, receipt.property, payer);
        for (claim_txid, _) in selected {
            crate::journal_move::move_pending_once(
                &mut self.pending_net_settlements,
                &mut self.swept_net_settlements,
                claim_txid,
            )
            .map_err(|_| LedgerError::MalformedSettlementJournal)?;
        }
        self.king_settlements.insert(txid, receipt);
        Ok(())
    }

    fn verify_settlement_journal(&self) -> Result<(), LedgerError> {
        let total = self
            .pending_net_settlements
            .len()
            .checked_add(self.swept_net_settlements.len())
            .ok_or(LedgerError::ArithmeticOverflow)?;
        if total > MAX_SETTLEMENT_RECORDS || self.king_settlements.len() > MAX_SETTLEMENT_RECORDS {
            return Err(LedgerError::SettlementJournalFull);
        }
        let mut refs = BTreeSet::new();
        for (txid, claim) in self
            .pending_net_settlements
            .iter()
            .chain(self.swept_net_settlements.iter())
        {
            if !self.processed.contains(txid)
                || claim.trade_reference == *txid
                || claim.trade_reference == TxId::from_bytes([0; 32])
                || claim.height == 0
                || claim.expiry_height < claim.height
                || claim.amount_atoms == 0
                || !self.properties.contains_key(&claim.property)
                || self.channel_owner(&claim.channel, ChannelSide::A).is_none()
                || self.channel_owner(&claim.channel, ChannelSide::B).is_none()
                || !refs.insert((claim.channel.clone(), claim.trade_reference))
            {
                return Err(LedgerError::MalformedSettlementJournal);
            }
        }
        if self
            .pending_net_settlements
            .keys()
            .any(|id| self.swept_net_settlements.contains_key(id))
        {
            return Err(LedgerError::MalformedSettlementJournal);
        }
        let mut covered = BTreeSet::new();
        for (sweep_id, receipt) in &self.king_settlements {
            if !self.processed.contains(sweep_id)
                || receipt.claim_txids.is_empty()
                || receipt.claim_txids.len() > MAX_SETTLEMENT_RECORDS
                || receipt.start_height == 0
                || receipt.end_height < receipt.start_height
                || receipt.end_height > receipt.settled_at
            {
                return Err(LedgerError::MalformedSettlementJournal);
            }
            let mut claims = Vec::with_capacity(receipt.claim_txids.len());
            for claim_id in &receipt.claim_txids {
                if !covered.insert(*claim_id) {
                    return Err(LedgerError::MalformedSettlementJournal);
                }
                let claim = self
                    .swept_net_settlements
                    .get(claim_id)
                    .ok_or(LedgerError::MalformedSettlementJournal)?;
                claims.push((*claim_id, claim));
            }
            if calculate_settlement_preview(
                &receipt.channel,
                receipt.property,
                receipt.start_height,
                receipt.end_height,
                receipt.settled_at,
                &claims,
            )? != receipt.preview
            {
                return Err(LedgerError::MalformedSettlementJournal);
            }
        }
        if covered.len() != self.swept_net_settlements.len() {
            return Err(LedgerError::MalformedSettlementJournal);
        }
        Ok(())
    }

    fn append_settlement_snapshot(&self, out: &mut Vec<u8>) -> Result<(), SnapshotError> {
        for claims in [&self.pending_net_settlements, &self.swept_net_settlements] {
            out.extend_from_slice(&(claims.len() as u32).to_be_bytes());
            for (txid, claim) in claims {
                out.extend_from_slice(&txid.as_bytes());
                push_account(out, &claim.channel);
                out.extend_from_slice(&claim.property.get().to_be_bytes());
                out.push(claim.payer_side.tag());
                out.extend_from_slice(&claim.amount_atoms.to_be_bytes());
                out.extend_from_slice(&claim.trade_reference.as_bytes());
                out.extend_from_slice(&claim.height.to_be_bytes());
                out.extend_from_slice(&claim.expiry_height.to_be_bytes());
            }
        }
        out.extend_from_slice(&(self.king_settlements.len() as u32).to_be_bytes());
        for (txid, receipt) in &self.king_settlements {
            out.extend_from_slice(&txid.as_bytes());
            push_account(out, &receipt.channel);
            out.extend_from_slice(&receipt.property.get().to_be_bytes());
            out.extend_from_slice(&receipt.start_height.to_be_bytes());
            out.extend_from_slice(&receipt.end_height.to_be_bytes());
            out.extend_from_slice(&receipt.settled_at.to_be_bytes());
            out.extend_from_slice(&receipt.preview.root);
            out.extend_from_slice(&receipt.preview.count.to_be_bytes());
            out.push(receipt.preview.payer_side.tag());
            out.extend_from_slice(&receipt.preview.net_atoms.to_be_bytes());
            out.extend_from_slice(&(receipt.claim_txids.len() as u32).to_be_bytes());
            for id in &receipt.claim_txids {
                out.extend_from_slice(&id.as_bytes());
            }
        }
        Ok(())
    }

    fn read_settlement_snapshot(
        &mut self,
        reader: &mut crate::codec::Cursor<'_>,
    ) -> Result<(), SnapshotError> {
        for settled in [false, true] {
            let count = snapshot_count(reader)?;
            let mut previous = None;
            for _ in 0..count {
                let txid = TxId::from_bytes(reader.fixed::<32>().map_err(snapshot_codec_error)?);
                if previous.is_some_and(|p| txid <= p) {
                    return Err(SnapshotError::InvalidFormat("unsorted settlement claims"));
                }
                previous = Some(txid);
                let channel = read_account(reader)?;
                let property = PropertyId::try_from(reader.u32().map_err(snapshot_codec_error)?)
                    .map_err(SnapshotError::InvalidLedger)?;
                let side = ChannelSide::from_tag(reader.u8().map_err(snapshot_codec_error)?)?;
                let amount_atoms = reader.u128().map_err(snapshot_codec_error)?;
                let trade_reference =
                    TxId::from_bytes(reader.fixed::<32>().map_err(snapshot_codec_error)?);
                let height = reader.u64().map_err(snapshot_codec_error)?;
                let expiry_height = reader.u64().map_err(snapshot_codec_error)?;
                let claim = NetClaim {
                    channel,
                    property,
                    payer_side: side,
                    amount_atoms,
                    trade_reference,
                    height,
                    expiry_height,
                };
                if settled {
                    self.swept_net_settlements.insert(txid, claim);
                } else {
                    self.pending_net_settlements.insert(txid, claim);
                }
            }
        }
        let count = snapshot_count(reader)?;
        let mut previous = None;
        for _ in 0..count {
            let txid = TxId::from_bytes(reader.fixed::<32>().map_err(snapshot_codec_error)?);
            if previous.is_some_and(|p| txid <= p) {
                return Err(SnapshotError::InvalidFormat("unsorted king settlements"));
            }
            previous = Some(txid);
            let channel = read_account(reader)?;
            let property = PropertyId::try_from(reader.u32().map_err(snapshot_codec_error)?)
                .map_err(SnapshotError::InvalidLedger)?;
            let start_height = reader.u64().map_err(snapshot_codec_error)?;
            let end_height = reader.u64().map_err(snapshot_codec_error)?;
            let settled_at = reader.u64().map_err(snapshot_codec_error)?;
            let root = reader.fixed::<32>().map_err(snapshot_codec_error)?;
            let count = reader.u32().map_err(snapshot_codec_error)?;
            let payer_side = ChannelSide::from_tag(reader.u8().map_err(snapshot_codec_error)?)?;
            let net_atoms = reader.u128().map_err(snapshot_codec_error)?;
            let ids_count = snapshot_count(reader)?;
            let mut prior = None;
            let mut ids = Vec::with_capacity(ids_count);
            for _ in 0..ids_count {
                let claim_id =
                    TxId::from_bytes(reader.fixed::<32>().map_err(snapshot_codec_error)?);
                if prior.is_some_and(|id| claim_id <= id) {
                    return Err(SnapshotError::InvalidFormat("unsorted king references"));
                }
                prior = Some(claim_id);
                ids.push(claim_id);
            }
            self.king_settlements.insert(
                txid,
                KingReceipt {
                    channel,
                    property,
                    start_height,
                    end_height,
                    settled_at,
                    preview: SettlementPreview {
                        root,
                        count,
                        payer_side,
                        net_atoms,
                    },
                    claim_txids: ids,
                },
            );
        }
        Ok(())
    }
}
