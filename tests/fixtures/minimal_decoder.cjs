// Fake reference for testing the comparison harness only.
// This is NOT the TradeLayer historical JavaScript implementation.
module.exports = {
  decodeActivateTradeLayer(body) {
    const [list, hash] = body.split(',');
    return {txTypesToActivate: list.split(';').map(Number), codeHash: hash};
  },
  decodeTokenIssue(body) {
    const parts = body.split(',');
    return {initialAmount: parseInt(parts[0], 36), ticker: parts[1], managed: parts[3] === '1'};
  },
  decodeSend(body) {
    const parts = body.split(';');
    if (parts[0] === '1') return {sendAll: true, address: parts[1]};
    const decimal = parts[3].endsWith('~');
    const quantity = parseInt(decimal ? parts[3].slice(0, -1) : parts[3], 36);
    return {sendAll: false, address: parts[1], propertyIds: parseInt(parts[2], 36), amounts: decimal ? quantity / 1e8 : quantity};
  },
  decodeGrantManagedToken(body) {
    const parts = body.split(',');
    return {propertyId: parseInt(parts[0], 36), amountGranted: parseInt(parts[1], 36) / 1e8, addressToGrantTo: parts[2]};
  },
  decodeRedeemManagedToken(body) {
    const parts = body.split(',');
    return {propertyId: parseInt(parts[0], 36), amountDestroyed: parseInt(parts[1], 36) / 1e8};
  }
};
