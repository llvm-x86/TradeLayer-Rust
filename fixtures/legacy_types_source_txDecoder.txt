// txDecoder.js
const BigNumber = require('bignumber.js');
const base94 = require('./base94.js')
const base256 = require('./base256.js')

const Decode = {
    decodeAmount : (encoded) => {
        const isDecimal = encoded.endsWith('~'); // Check for decimal flag `~`
        const numStr = isDecimal ? encoded.slice(0, -1) : encoded;
        const value = new BigNumber(parseInt(numStr, 36));

        return isDecimal ? value.div(1e8).toNumber() : value.toNumber();
    },
   // Decode Activate TradeLayer Transaction
     decodeActivateTradeLayer(payload) {
        const parts = payload.split(',');
        console.log('payload '+payload)
        console.log('parts '+parts[0])
        console.log('split array '+parts[0].split(';'))
        // Decode txType(s)
        const txTypes = parts[0].includes(';') 
            ? parts[0].split(';').map(value => {
                const num = parseInt(value, 10); // Parse as integer
                return isNaN(num) ? null : num; // Handle invalid entries
            })
            : [parseInt(parts[0], 10)]; // Single value case

        // Decode codeHash
        const decodedHash = parts[1] 

        console.log('Decoded txTypes:', txTypes, 'Decoded Hash:', decodedHash);

        return {
            txTypesToActivate: txTypes.filter(txType => txType !== null), // Remove invalid txTypes
            codeHash: decodedHash
        };
    },



    // Decode Token Issue Transaction
    decodeTokenIssue: (payload) => {
        const parts = payload.split(',');
        return {
            initialAmount: parts[0] ? parseInt(parts[0], 36) : 0,
            ticker: parts[1] || '',
            whitelists: parts[2] ? parts[2].split(';').map(val => parseInt(val, 36)) : [],
            managed: parts[3] === '1',
            backupAddress: parts[4] || '',
            nft: parts[5] === '1',
            coloredCoinHybrid: parts[6] === '1',
            proceduralType: parts[7] ? parseInt(parts[7], 36) : null
        };
    },

        // Decode Send Transaction
    decodeSend(payload){
    const parts = payload.split(';');
    const sendAll = parts[0] === '1';
    const address = parts[1] || '';
    let isColoredOutput = false;

    // Helper function to decode amounts correctly
    decodeAmount = (encoded) => {
            const isDecimal = encoded.endsWith('~'); // Check for decimal flag `~`
            const numStr = isDecimal ? encoded.slice(0, -1) : encoded;
            const value = new BigNumber(parseInt(numStr, 36));
            if(isDecimal){console.log('decimal value encountered' +value)}
            return isDecimal ? value.div(1e8).toNumber() : value.toNumber();
        };

        if (sendAll) {
            return { sendAll, address };
        } else if (parts.length === 5 || parts.length === 4) {
            const propertyIds = parseInt(parts[2], 36) || 0;
            const amounts = decodeAmount(parts[3] || '0');
            isColoredOutput = parts[5] === '1';

            return { sendAll, address, propertyIds, amounts, isColoredOutput };
        } else if (parts[2].includes(',')) {
            const propertyIds = (parts[2] || '').split(',').map(id => parseInt(id, 36));
            const amounts = (parts[3] || '').split(',').map(decodeAmount);

            return {
                sendAll,
                propertyIds: propertyIds.map((id, index) => ({ propertyIds: id, amounts: amounts[index] }))
            };
        }
    },

    // Decode Property ID with error handling
    decodePropertyId(encodedPropertyId) {
        if (encodedPropertyId.startsWith('s')) {
            const trimmedEncodedPropertyId = encodedPropertyId.substring(1);
            let encodedCollateralId, encodedContractId;
            if (trimmedEncodedPropertyId.includes('-')) {
                const parts = trimmedEncodedPropertyId.split('-');
                if (parts.length === 2) {
                    [encodedCollateralId, encodedContractId] = parts;
                } else if (parts.length === 3 && parts[0] === '') {
                    encodedCollateralId = parts[1];
                    encodedContractId = parts[2];
                } else {
                    return `s-NaN-NaN`;
                }
            } else {
                return `s-NaN-NaN`;
            }
            const collateralId = parseInt(encodedCollateralId, 36);
            const contractId = parseInt(encodedContractId, 36);
            if (isNaN(collateralId) || isNaN(contractId)) {
                return `s-NaN-NaN`;
            }
            return `s-${collateralId}-${contractId}`;
        } else {
            const result = parseInt(encodedPropertyId, 36);
            return isNaN(result) ? NaN : result;
        }
    },

    // Decode Trade Token for UTXO Transaction
    decodeTradeTokenForUTXO: (payload) => {
        const parts = payload.split(',');
        return {
            propertyId: Decode.decodePropertyId(parts[0] || ''),
            amount: new BigNumber(parts[1] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber(),
            columnA: parts[2] === "1",
            satsExpected: parseInt(parts[3],36),
            tokenOutput: parseInt(parts[4] || '0'),
            payToAddress: parseInt(parts[5] || '0'),
            isColoredOutput: parts[6] === "1",
            tagWithdraw: parts[7] || null
        };
    },

    // Decode Commit Token Transaction
    decodeCommitToken: (payload) => {
        const parts = payload.split(',');
        let propertyId = Decode.decodePropertyId(parts[0] || '');
        let amount = new BigNumber(parts[1] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber();
        
        // Handle channelAddress or reference
        let channelAddress = '';
        let ref = false;
        if (parts[2].startsWith('ref:')) {
            ref = parts[2].split(':')[1];
        } else {
            channelAddress = parts[2] || '';
        }

        // Decode payEnabled
        let payEnabled = parts[3] === '1';

        // Decode clearLists
        let clearLists = [];
        if (parts[4] && parts[4].startsWith('[') && parts[4].endsWith(']')) {
            clearLists = parts[4].slice(1, -1).split(',').map(num => parseInt(num, 36));
        } else if (parts[4]) {
            clearLists = [parseInt(parts[4], 36)];
        }

        const isColoredOutput = parts[5] ==='1'
        const commitClearlistId = parts[6] !== undefined && parts[6] !== ''
            ? parseInt(parts[6], 36)
            : null;

        return {
            propertyId,
            amount,
            channelAddress,
            ref,
            payEnabled,
            clearLists,
            isColoredOutput,
            commitClearlistId
        };
    },


    // Decode On-chain Token for Token Transaction
    decodeOnChainTokenForToken: (payload) => {
        const parts = payload.split(',');
        return {
            propertyIdOffered: Decode.decodePropertyId(parts[0] || ''),
            propertyIdDesired: Decode.decodePropertyId(parts[1] || ''),
            amountOffered: new BigNumber(parts[2] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber(),
            amountExpected: new BigNumber(parts[3] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber(),
            stop: parts[4] === "1",
            post: parts[5] === "1"
        };
    },
   
    // Decode Cancel Order Transaction with guards
    decodeCancelOrder: (encodedTx) => {
        const elements = encodedTx.split(',');
        let isContract = elements[0];
        const cancelParams = {};
        let offeredPropertyId, desiredPropertyId, cancelAll;

        if (isContract == 1) {
            isContract = true;
            offeredPropertyId = Decode.decodePropertyId(elements[1] || '');
            cancelAll = parseInt(elements[2] || '0', 36);
            if (elements[3] && elements[3].length > 20) {
                cancelParams.txid = elements[3];
            } else {
                cancelParams.price = elements[3] || '0';
                cancelParams.side = elements[4] || '0';
            }
        } else {
            isContract = false;
            offeredPropertyId = Decode.decodePropertyId(elements[1] || '');
            desiredPropertyId = Decode.decodePropertyId(elements[2] || '');
            cancelAll = parseInt(elements[3] || '0', 36);
            if (elements[4] && elements[4].length > 20) {
                cancelParams.txid = elements[4];
            } else {
                const priceDecoded = new BigNumber(elements[3] || '0').dividedBy(8).toNumber();
                cancelParams.price = priceDecoded;
                cancelParams.side = elements[5] || '0';
            }
        }
        return { isContract, offeredPropertyId, desiredPropertyId, cancelAll, cancelParams };
    },

    // Decode Create Whitelist Transaction
    decodeCreateWhitelist: (payload) => {
        const parts = payload.split(',');
        return {
            backupAddress: parts[0] || '',
            name: parts[1] || '',
            url: parts[2] || '',
            description: parts[3] || ''
        };
    },

        // Decode Update Admin Transaction
    decodeUpdateAdmin: (payload) => {
        const parts = payload.split(',');
        return {
            newAddress: parts[0] || '',
            whitelist: parts[1] === '1',
            oracle: parts[2] === '1',
            token: parts[3] === '1',
            id: parseInt(parts[4] || '0', 36),
            updateBackup: parts[5] === '1'
        };
    },

    // Decode Issue Attestation Transaction
    decodeIssueOrRevokeAttestation: (payload) => {
        const parts = payload.split(',');
        console.log('decoding attestation '+JSON.stringify(parts))
        return {
            revoke: parts[0] === '1',
            id: parseInt(parts[1] || '0', 36),
            targetAddress: parts[2] || '',
            metaData: parts[3].toUpperCase() || ''
        };
    },

    // Decode AMM Pool Transaction
    decodeAMMPool: (payload) => {
        const parts = payload.split(',');
        return {
            isRedeem: parts[0] === '1',
            isContract: parts[1] === '1',
            id: Decode.decodePropertyId(parts[2] || ''),
            amount: parseInt(parts[3] || '0', 36),
            id2: Decode.decodePropertyId(parts[4] || ''),
            amount2: parseInt(parts[5] || '0', 36),
            optionsMaker: parseInt(parts[7] || '0'),   // AMM id
            optionsTaker: parseInt(parts[8] || '0'),   // AMM id
            strategyBlob: parts[9] || ''               // free-form blob
        };
    },


    // Decode Grant Managed Token Transaction
    decodeGrantManagedToken: (payload) => {
        const parts = payload.split(',');
        const trailingHash = parts[7] || '';
        const legacyHash = parts[3] || '';
        return {
            propertyId: Decode.decodePropertyId(parts[0] || ''),
            amountGranted: new BigNumber(parts[1] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber(),
            addressToGrantTo: parts[2] || '',
            dlcHash: trailingHash || legacyHash,
            dlcTemplateId: parts[4] || '',
            dlcContractId: parts[5] || '',
            settlementState: parts[6] || ''
        };
    },

    // Decode Redeem Managed Token Transaction
    decodeRedeemManagedToken: (payload) => {
        const parts = payload.split(',');
        return {
            propertyId: Decode.decodePropertyId(parts[0] || ''),
            amountDestroyed: new BigNumber(parts[1] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber(),
            dlcTemplateId: parts[2] || '',
            dlcContractId: parts[3] || '',
            settlementState: parts[4] || ''
        };
    },

    // Decode Create Oracle Transaction
    decodeCreateOracle: (payload) => {
        const parts = payload.split(',');
        return {
            ticker: parts[0] || '',
            url: parts[1] || '',
            backupAddress: parts[2] || '',
            whitelists: parts[3] ? parts[3].split(';').map(val => parseInt(val, 36)) : [],
            lag: parseInt(parts[4] || '0', 36)
        };
    },

    // Decode Publish Oracle Data Transaction
    decodePublishOracleData: (payload) => {
        const parts = payload.split(',');
        let data = {
            oracleId: parseInt(parts[0] || '0', 36), // Decode oracleId as the first part
            price: parseInt(parts[1] || '0', 36)     // Adjust indices for other parts
        };
        data.price = new BigNumber(data.price).div(1e4).decimalPlaces(4).toNumber()
        if (parts[2]) {
            data.high = parseInt(parts[2], 36);
        }
        if (parts[3]) {
            data.low = parseInt(parts[3], 36);
        }
        if (parts[4]) {
            data.close = parseInt(parts[4], 36);
        }
        return data;
    },

    // Decode Close Oracle Transaction
    decodeCloseOracle: (payload) => {
        return parseInt(payload || '0', 36); // No parameters
    },

    // Decode Create Future Contract Series Transaction
    decodeCreateFutureContractSeries: (payload) => {
        const parts = payload.split(',');

        // Check if the contract is native or not
        const isNative = parts[0] === '1';

        // Initialize onChainDataParts
        let onChainDataParts = [];

        // Parse onChainData only if the contract is not native
        if (!isNative && parts[2]) {
            onChainDataParts = parts[2].split(';').map(pair => 
                pair.split(':').map(val => val ? parseInt(val, 36) : null)
            );
        }

        console.log('decoding create contract notionalValue '+parts[4])

        return {
            native: isNative,
            underlyingOracleId: parseInt(parts[1] || '0', 36),
            onChainData: onChainDataParts,
            notionalPropertyId: Decode.decodePropertyId(parts[3] || ''),
            notionalValue: parseInt(parts[4] || '0',36), // Assuming notionalValue should be a float
            collateralPropertyId: Decode.decodePropertyId(parts[5] || ''),
            leverage: parseFloat(parts[6] || '0'), // Assuming leverage should be a float
            expiryPeriod: parts[7] ? parseInt(parts[7], 36) : null,
            series: parts[8] ? parseInt(parts[8], 36) : null,
            inverse: parts[9] === '1',
            fee: parts[10] === '1'
        };
    },

    // Decode Exercise Derivative Transaction
    decodeExerciseDerivative: (payload) => {
        const [derivativeContractId, amount] = payload.split(',');
        return {
            derivativeContractId: parseInt(derivativeContractId || '0', 36),
            amount: parseInt(amount || '0', 36)
        };
    },

     // **✅ Decode Trade Contract Onchain**
    decodeTradeContractOnchain: (payload) => {
        const parts = payload.split(',');
        const price = parseInt(parts[1], 36)||0
        console.log('price in decode on chain contract '+price )
        return {
            contractId: parseInt(parts[0] || '0', 36),
            price: new BigNumber(price).div(1e8).toNumber(),
            amount: parseInt(parts[2] || '0', 36),
            sell: parts[3] === '1',
            insurance: parts[4] === '1',
            reduce: parts[5] === '1',
            post: parts[6] === '1',
            stop: parts[7] === '1'
        };
    },

    // Decode Trade Contract in Channel Transaction
    decodeTradeContractChannel: (payload) => {
        const parts = payload.split(',');
        const price = parseInt(parts[1], 36)||0
        return {
            contractId: parseInt(parts[0] || '0', 36),
            price: new BigNumber(price).div(1e8).toNumber(),
            amount: parseInt(parts[2] || '0', 36),
            columnAIsSeller: parts[3] === '1',
            expiryBlock: parseInt(parts[4] || '0', 36),
            insurance: parts[5] === '1',
            columnAIsMaker: parts[6] === '1'
        };
    },

    // Decode Trade Tokens in Channel Transaction
    decodeTradeTokensChannel: (payload) => {
        const parts = payload.split(',');
        return {
            propertyIdOffered: Decode.decodePropertyId(parts[0] || ''),
            propertyIdDesired: Decode.decodePropertyId(parts[1] || ''),
            amountOffered: new BigNumber(parts[2] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber(),
            amountDesired: new BigNumber(parts[3] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber(),
            columnAIsOfferer: parts[4] === '1',
            expiryBlock: parseInt(parts[5] || '0', 36),
            columnAIsMaker: parts[6] === '1',
            Id1ColoredOutput: parts[7]=== '1',
            Id2ColoredOutput: parts[8]=== '1'
        };
    },

    // Decode Withdrawal Transaction
     decodeWithdrawal: (payload, decodedTx) => {
      const parts = payload.split(',');
      let channelAddress = '';
      let ref = false;

      if (parts[4]?.startsWith('ref:')) {
        const n = parseInt(parts[4].split(':')[1], 10);
        ref = Number.isFinite(n) ? n : false;
      } else {
        channelAddress = parts[4] || '';
      }

      return {
        withdrawAll: parts[0] === '1',
        propertyId: Decode.decodePropertyId(parts[1] || ''),
        amount: new BigNumber(parts[2] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber(),
        column: parts[3] === '1',
        channelAddress,
        ref,
      };
    },


    // Decode Transfer Transaction
    decodeTransfer: (payload) => {
        const parts = payload.split(',');
        let propertyId = Decode.decodePropertyId(parts[0] || '');
        let amount = new BigNumber(parts[1] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber();
        let isColumnA = parts[2] === '1';
        
        // Handle destinationAddr or reference
        let toChannelAddress = '';
        let ref = false;
        if (parts[3].startsWith('ref:')) {
            ref = parts[3].split(':')[1];
        } else {
            toChannelAddress = parts[3] || '';
        }

        // Decode pay and payRef
        let pay = parts[4] === '1';
        let payRef = parts[5] || '';

        return {
            propertyId,
            amount,
            isColumnA,
            toChannelAddress,
            ref,
            pay,
            payRef
        };
    },

    // Decode Settle Channel PNL Transaction
   decodeSettleChannelPNL: (payload) => {
        const parts = payload.split(',');

        return {
            txidNeutralized1: base256.base256ToHex(parts[0] || ''), // Decode from Base 256 to Hex
            txidNeutralized2: base256.base256ToHex(parts[1] || ''), // Decode from Base 256 to Hex
            markPrice: parseFloat(base94.fromBase94(parts[2] || '')), // Decode from Base 94 to decimal
            close: parts[3] === '1',
            columnAIsSeller: parts[4]=== '1',
            columnAIsMaker: parts[5]==='1',
            macroBatch: parts[6] ==='1'
            // Boolean flag for closing trade
        };
    },

   // Decode Mint Synthetic Transaction
    decodeMintSynthetic: (payload) => {
        const parts = payload.split(',');
        return {
            propertyId: Decode.decodePropertyId(parts[0] || ''),
            contractId: parseInt(parts[1] || '0', 36),
            amount: new BigNumber(parts[2] || '0', 36)
                .div(1e8)
                .decimalPlaces(8, BigNumber.ROUND_DOWN)
                .toNumber(),
        };
    },

    // Decode Redeem Synthetic Transaction
    decodeRedeemSynthetic: (payload) => {
        const parts = payload.split(',');
        return {
            propertyId: parseInt(parts[0] || '0', 36),
            contractId: parseInt(parts[1] || '0', 36),
            amount: new BigNumber(parts[2] || '0', 36)
                .div(1e8)
                .decimalPlaces(8, BigNumber.ROUND_DOWN)
                .toNumber(),
        };
    },

    // Decode Pay to Tokens Transaction
    decodePayToTokens: (payload) => {
        const parts = payload.split(',');
        return {
            propertyIdTarget: parseInt(parts[0] || '0', 36),
            propertyIdUsed: parseInt(parts[1] || '0', 36),
            amount: new BigNumber(parts[2] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber(),
        };
    },

    decodeOptionTrade: (payload) => {
        const parts = payload.split(',');

        const ticker = parts[0];
        const result = {
            contractId: ticker,
            ticker,                                                // backward compatibility
            price: Decode.decodeAmount(parts[1] || '0'),
            amount: parseInt(parts[2] || '0', 36),
            columnAIsSeller: parts[3]=== '1',
            expiryBlock: parseInt(parts[4] || '0', 36),
            columnAIsMaker: parts[5] === '1'
        };

        if (parts.length > 6) {
            result.comboTicker = parts[6];
            result.comboPrice = Decode.decodeAmount(parts[7] || '0');
            result.comboAmount = parseInt(parts[8] || '0', 36);
        }

        return result;
    },

        // Type 23: Settle Channel PNL (polymorphic)
    decodeSettleChannelPNL: (payload) => {
        const parts = payload.split(',');
        return {
            txidNeutralized1: parts[0] || '',
            txidNeutralized2: parts[1] || '',
            markPrice: parseFloat(base94.fromBase94(parts[2] || '0')),
            settleType: parseInt(parts[3] || '0'),
            columnAIsSeller: parts[4] === '1',
            columnAIsMaker: parts[5] === '1',
            netAmount: parseFloat(base94.fromBase94(parts[6] || '0')),
            expiryBlock: parseInt(base94.fromBase94(parts[7] || '0')),
            // Optional king-style fields for settleType=3 routed through tx23
            blockStart: parts[8] ? parseInt(base94.fromBase94(parts[8])) : undefined,
            blockEnd: parts[9] ? parseInt(base94.fromBase94(parts[9])) : undefined,
            propertyId: parts[10] ? parseInt(base94.fromBase94(parts[10])) : undefined,
            aPaysBDirection: parts[11] ? (parts[11] === '1') : undefined,
            channelRoot: parts[12] || undefined,
            totalContracts: parts[13] ? parseInt(base94.fromBase94(parts[13])) : undefined,
            neutralizedCount: parts[14] ? parseInt(base94.fromBase94(parts[14])) : undefined
        };
    },

    // Type 31: King Settlement
    decodeKingSettle: (payload) => {
            const parts = payload.split(',');
            return {
                blockStart: parseInt(base94.fromBase94(parts[0] || '0')),
                blockEnd: parseInt(base94.fromBase94(parts[1] || '0')),
                propertyId: parseInt(base94.fromBase94(parts[2] || '0')),
                netAmount: parseFloat(base94.fromBase94(parts[3] || '0')),
                aPaysBDirection: parts[4] === '1',
                channelRoot: parts[5] || '',
                totalContracts: parseInt(base94.fromBase94(parts[6] || '0')),
                neutralizedCount: parseInt(base94.fromBase94(parts[7] || '0'))
            };
        },

    // Type 30: Oracle stake/fraud/relay
    decodeStakeFraudProof: (payload) => {
        const parts = payload.split(',');
        return {
            action: parseInt(parts[0] || '0', 36), // 0=stake,1=fraud,2=relay
            oracleId: parseInt(parts[1] || '0', 36),
            stakedPropertyId: parseInt(parts[2] || '0', 36),
            amount: new BigNumber(parts[3] || '0', 36).div(1e8).decimalPlaces(8, BigNumber.ROUND_DOWN).toNumber(),
            accusedAddress: parts[4] || '',
            evidenceHash: parts[5] || '',
            relayType: parseInt(parts[6] || '0', 36),
            stateHash: parts[7] || '',
            dlcRef: parts[8] || '',
            settlementState: parts[9] || '',
            relayBlob: parts[10] || '',
            autoRoll: parts[11] === '1',
            nextDlcRef: parts[12] || ''
        };
    },

    decodeBatchMoveZkRollup: (payload) =>{
       return { ordinalRevealJSON: payload };
    },

    // Decode Publish New Transaction Type
    decodePublishNewTx: (payload) => {
        return { ordinalRevealJSON: payload };
    },

    // Decode Create Derivative of LRC20 or RGB
    decodeColoredCoin: (payload) => {
        const parts = payload.split(',');

        // Ensure there are enough parts to avoid undefined access
        return {
            encodeDecodeRecode: parseInt(parts[0], 10), // Assuming it's a numeric identifier
            propertyId: parseInt(parts[1], 36), // The TL account token being encoded
            satsRatio: parseInt(parts[2], 36), // How many sats of the account token
            homeAddress: parts[3] || '' // Optional address, defaults to empty string if not provided
        };
    },

    // Decode Mint Colored Coin
    decodeAbstractionBridge: (payload) => {
        const parts = payload.split(',');
        return {
            propertyId: parseInt(parts[0], 36),
            amount: parseInt(parts[1], 36)
        };
    }

}

// ... continue decoding functions for the rest of the transactions ...

module.exports = Decode
