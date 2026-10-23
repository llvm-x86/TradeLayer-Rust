// Channel escrow is a deliberately bounded, simulation-grade ledger extension.
// Account identities and column selection are claims until a chain adapter
// authenticates transaction signatures, UTXOs and legacy channel rules.
// Included from ledger.rs so the book cannot bypass supply reconciliation.

/// A channel's two distinct ownership columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChannelSide {
    A,
    B,
}
impl ChannelSide {
    fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
    fn tag(self) -> u8 {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }
    fn from_tag(tag: u8) -> Result<Self, SnapshotError> {
        match tag {
            0 => Ok(Self::A),
            1 => Ok(Self::B),
            _ => Err(SnapshotError::InvalidFormat("invalid channel side")),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ChannelState {
    owner_a: Option<AccountId>,
    owner_b: Option<AccountId>,
    escrow_a: BTreeMap<PropertyId, u128>,
    escrow_b: BTreeMap<PropertyId, u128>,
}
impl ChannelState {
    fn owner(&self, side: ChannelSide) -> Option<&AccountId> {
        match side {
            ChannelSide::A => self.owner_a.as_ref(),
            ChannelSide::B => self.owner_b.as_ref(),
        }
    }
    fn escrow(&self, side: ChannelSide) -> &BTreeMap<PropertyId, u128> {
        match side {
            ChannelSide::A => &self.escrow_a,
            ChannelSide::B => &self.escrow_b,
        }
    }
    fn escrow_mut(&mut self, side: ChannelSide) -> &mut BTreeMap<PropertyId, u128> {
        match side {
            ChannelSide::A => &mut self.escrow_a,
            ChannelSide::B => &mut self.escrow_b,
        }
    }
    fn atoms(&self, side: ChannelSide, id: PropertyId) -> u128 {
        self.escrow(side).get(&id).copied().unwrap_or(0)
    }
    fn put(&mut self, side: ChannelSide, id: PropertyId, atoms: u128) {
        if atoms == 0 {
            self.escrow_mut(side).remove(&id);
        } else {
            self.escrow_mut(side).insert(id, atoms);
        }
    }
    fn claim(
        &mut self,
        channel: &AccountId,
        side: ChannelSide,
        owner: &AccountId,
    ) -> Result<(), LedgerError> {
        if self.owner(side.other()) == Some(owner) {
            return Err(LedgerError::ChannelSideOccupied(channel.clone()));
        }
        match self.owner(side) {
            Some(prior) if prior != owner => Err(LedgerError::ChannelSideOccupied(channel.clone())),
            Some(_) => Ok(()),
            None => {
                match side {
                    ChannelSide::A => self.owner_a = Some(owner.clone()),
                    ChannelSide::B => self.owner_b = Some(owner.clone()),
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingWithdrawal {
    channel: AccountId,
    side: ChannelSide,
    owner: AccountId,
    id: PropertyId,
    atoms: u128,
    unlock_height: u64,
}

impl Ledger {
    /// Escrow remains property supply, owned by exactly one column participant.
    pub fn channel_atoms(&self, id: PropertyId, channel: &AccountId, side: ChannelSide) -> u128 {
        self.channels.get(channel).map_or(0, |c| c.atoms(side, id))
    }
    pub fn channel_owner(&self, channel: &AccountId, side: ChannelSide) -> Option<&AccountId> {
        self.channels.get(channel).and_then(|c| c.owner(side))
    }
    pub fn pending_withdrawal_count(&self) -> usize {
        self.pending_withdrawals.len()
    }
    pub fn pending_withdrawal_atoms(&self, id: PropertyId, owner: &AccountId) -> u128 {
        self.pending_withdrawals
            .values()
            .filter(|p| p.id == id && &p.owner == owner)
            .fold(0u128, |sum, p| sum.saturating_add(p.atoms))
    }

    fn commit_channel(
        &mut self,
        id: PropertyId,
        from: AccountId,
        channel: AccountId,
        side: ChannelSide,
        amount_atoms: u128,
    ) -> Result<(), LedgerError> {
        self.existing_property(id)?;
        if amount_atoms == 0 {
            return Err(LedgerError::ZeroAmount);
        }
        let balance = self.available_atoms(id, &from);
        if balance < amount_atoms {
            return Err(LedgerError::InsufficientAvailable {
                property: id,
                available: balance,
                requested: amount_atoms,
            });
        }
        let state = self.channels.entry(channel.clone()).or_default();
        state.claim(&channel, side, &from)?;
        let (debit_after, credit_after) =
            checked_move_for_property(id, balance, state.atoms(side, id), amount_atoms)?;
        state.put(side, id, credit_after);
        self.put_balance(id, from, debit_after);
        Ok(())
    }

    fn request_channel_withdrawal(
        &mut self,
        txid: TxId,
        request: PendingWithdrawal,
    ) -> Result<(), LedgerError> {
        let id = request.id;
        self.existing_property(id)?;
        if request.atoms == 0 {
            return Err(LedgerError::ZeroAmount);
        }
        if request.unlock_height == 0 {
            return Err(LedgerError::InvalidWithdrawalHeight);
        }
        let state = self
            .channels
            .get_mut(&request.channel)
            .ok_or_else(|| LedgerError::UnknownChannel(request.channel.clone()))?;
        if state.owner(request.side) != Some(&request.owner) {
            return Err(LedgerError::ChannelSideMismatch(request.channel.clone()));
        }
        let balance = state.atoms(request.side, id);
        if balance < request.atoms {
            return Err(LedgerError::InsufficientAvailable {
                property: id,
                available: balance,
                requested: request.atoms,
            });
        }
        state.put(request.side, id, balance - request.atoms);
        self.pending_withdrawals.insert(txid, request);
        Ok(())
    }

    fn transfer_channel(
        &mut self,
        id: PropertyId,
        from_channel: AccountId,
        from_side: ChannelSide,
        to_channel: AccountId,
        to_side: ChannelSide,
        amount_atoms: u128,
    ) -> Result<(), LedgerError> {
        self.existing_property(id)?;
        if amount_atoms == 0 {
            return Err(LedgerError::ZeroAmount);
        }
        if from_channel == to_channel {
            return Err(LedgerError::SameChannel);
        }
        let from = self
            .channels
            .get(&from_channel)
            .ok_or_else(|| LedgerError::UnknownChannel(from_channel.clone()))?;
        let owner = from
            .owner(from_side)
            .ok_or_else(|| LedgerError::ChannelSideMismatch(from_channel.clone()))?
            .clone();
        let balance = from.atoms(from_side, id);
        if balance < amount_atoms {
            return Err(LedgerError::InsufficientAvailable {
                property: id,
                available: balance,
                requested: amount_atoms,
            });
        }
        let to = self.channels.entry(to_channel.clone()).or_default();
        to.claim(&to_channel, to_side, &owner)?;
        let (debit_after, credit_after) =
            checked_move_for_property(id, balance, to.atoms(to_side, id), amount_atoms)?;
        to.put(to_side, id, credit_after);
        self.channels
            .get_mut(&from_channel)
            .expect("verified channel")
            .put(from_side, id, debit_after);
        Ok(())
    }

    /// Mature reserved withdrawals once the enclosing verified block reaches
    /// their height. Invoked after block operations, including empty TL blocks.
    /// Direct callers must supply a monotonically authenticated block height.
    pub fn settle_due_withdrawals_unverified(&mut self, height: u64) -> Result<usize, LedgerError> {
        if height == 0 {
            return Err(LedgerError::InvalidWithdrawalHeight);
        }
        let mut staged = self.clone();
        let due: Vec<TxId> = staged
            .pending_withdrawals
            .iter()
            .filter(|(_, withdrawal)| withdrawal.unlock_height <= height)
            .map(|(txid, _)| *txid)
            .collect();
        for txid in &due {
            let withdrawal = staged
                .pending_withdrawals
                .remove(txid)
                .expect("selected pending withdrawal");
            let prior = staged.available_atoms(withdrawal.id, &withdrawal.owner);
            let after = prior
                .checked_add(withdrawal.atoms)
                .ok_or(LedgerError::ArithmeticOverflow)?;
            staged.put_balance(withdrawal.id, withdrawal.owner, after);
        }
        staged.verify_conservation()?;
        *self = staged;
        Ok(due.len())
    }

    fn add_channel_supply(
        &self,
        sums: &mut BTreeMap<PropertyId, u128>,
        id: PropertyId,
        atoms: u128,
    ) -> Result<(), LedgerError> {
        if atoms == 0 {
            return Err(LedgerError::NonCanonicalZeroBalance(id));
        }
        if !self.properties.contains_key(&id) {
            return Err(LedgerError::BalanceWithoutProperty(id));
        }
        let sum = crate::checked_amounts::checked_supply_add(
            sums.get(&id).copied().unwrap_or(0),
            atoms,
        )
        .ok_or(LedgerError::ArithmeticOverflow)?;
        sums.insert(id, sum);
        Ok(())
    }

    fn add_channel_conservation(
        &self,
        sums: &mut BTreeMap<PropertyId, u128>,
    ) -> Result<(), LedgerError> {
        for (address, state) in &self.channels {
            if state.owner_a.is_some() && state.owner_a == state.owner_b {
                return Err(LedgerError::ChannelSideOccupied(address.clone()));
            }
            for side in [ChannelSide::A, ChannelSide::B] {
                for (&id, &atoms) in state.escrow(side) {
                    if state.owner(side).is_none() {
                        return Err(LedgerError::MalformedChannel(id));
                    }
                    self.add_channel_supply(sums, id, atoms)?;
                }
            }
        }
        for (txid, pending) in &self.pending_withdrawals {
            if pending.unlock_height == 0
                || !self.processed.contains(txid)
                || self.channel_owner(&pending.channel, pending.side) != Some(&pending.owner)
            {
                return Err(LedgerError::MalformedPendingWithdrawal(pending.id));
            }
            self.add_channel_supply(sums, pending.id, pending.atoms)?;
        }
        Ok(())
    }

    // Version 2 of the ledger snapshot appends the channel book and pending
    // withdrawal queue after the unchanged version-1 replay records.
    fn append_channel_snapshot(&self, out: &mut Vec<u8>) -> Result<(), SnapshotError> {
        let count = u32::try_from(self.channels.len())
            .map_err(|_| SnapshotError::InvalidFormat("too many channels"))?;
        out.extend_from_slice(&count.to_be_bytes());
        for (channel, record) in &self.channels {
            push_account(out, channel);
            for side in [ChannelSide::A, ChannelSide::B] {
                match record.owner(side) {
                    None => out.push(0),
                    Some(owner) => {
                        out.push(1);
                        push_account(out, owner);
                    }
                }
                let balances = record.escrow(side);
                if balances.len() > MAX_SNAPSHOT_ENTRIES {
                    return Err(SnapshotError::InvalidFormat("too many channel balances"));
                }
                out.extend_from_slice(&(balances.len() as u32).to_be_bytes());
                for (id, atoms) in balances {
                    out.extend_from_slice(&id.get().to_be_bytes());
                    out.extend_from_slice(&atoms.to_be_bytes());
                }
            }
        }
        let count = u32::try_from(self.pending_withdrawals.len())
            .map_err(|_| SnapshotError::InvalidFormat("too many withdrawals"))?;
        out.extend_from_slice(&count.to_be_bytes());
        for (txid, pending) in &self.pending_withdrawals {
            out.extend_from_slice(&txid.as_bytes());
            push_account(out, &pending.channel);
            out.push(pending.side.tag());
            push_account(out, &pending.owner);
            out.extend_from_slice(&pending.id.get().to_be_bytes());
            out.extend_from_slice(&pending.atoms.to_be_bytes());
            out.extend_from_slice(&pending.unlock_height.to_be_bytes());
        }
        Ok(())
    }

    fn read_channel_snapshot(
        &mut self,
        reader: &mut crate::codec::Cursor<'_>,
    ) -> Result<(), SnapshotError> {
        let count = snapshot_count(reader)?;
        let mut prev = None;
        for _ in 0..count {
            let channel = read_account(reader)?;
            if prev.as_ref().is_some_and(|id: &AccountId| &channel <= id) {
                return Err(SnapshotError::InvalidFormat("unsorted channels"));
            }
            prev = Some(channel.clone());
            let mut state = ChannelState::default();
            for side in [ChannelSide::A, ChannelSide::B] {
                let owner = match reader.u8().map_err(snapshot_codec_error)? {
                    0 => None,
                    1 => Some(read_account(reader)?),
                    _ => return Err(SnapshotError::InvalidFormat("channel owner flag")),
                };
                match side {
                    ChannelSide::A => state.owner_a = owner,
                    ChannelSide::B => state.owner_b = owner,
                }
                let count = snapshot_count(reader)?;
                let mut prior = None;
                for _ in 0..count {
                    let id = PropertyId::try_from(reader.u32().map_err(snapshot_codec_error)?)
                        .map_err(SnapshotError::InvalidLedger)?;
                    if prior.is_some_and(|v| id <= v) {
                        return Err(SnapshotError::InvalidFormat("unsorted channel balances"));
                    }
                    prior = Some(id);
                    let atoms = reader.u128().map_err(snapshot_codec_error)?;
                    if atoms == 0 {
                        return Err(SnapshotError::InvalidFormat("zero channel balance"));
                    }
                    state.escrow_mut(side).insert(id, atoms);
                }
            }
            self.channels.insert(channel, state);
        }
        let count = snapshot_count(reader)?;
        let mut previous: Option<TxId> = None;
        for _ in 0..count {
            let txid = TxId::from_bytes(reader.fixed::<32>().map_err(snapshot_codec_error)?);
            if previous.is_some_and(|p| txid <= p) {
                return Err(SnapshotError::InvalidFormat("unsorted pending withdrawals"));
            }
            previous = Some(txid);
            let channel = read_account(reader)?;
            let side = ChannelSide::from_tag(reader.u8().map_err(snapshot_codec_error)?)?;
            let owner = read_account(reader)?;
            let id = PropertyId::try_from(reader.u32().map_err(snapshot_codec_error)?)
                .map_err(SnapshotError::InvalidLedger)?;
            let atoms = reader.u128().map_err(snapshot_codec_error)?;
            let unlock_height = reader.u64().map_err(snapshot_codec_error)?;
            self.pending_withdrawals.insert(
                txid,
                PendingWithdrawal {
                    channel,
                    side,
                    owner,
                    id,
                    atoms,
                    unlock_height,
                },
            );
        }
        Ok(())
    }
}
