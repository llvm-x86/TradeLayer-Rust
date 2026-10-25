//! File 7/49: source txEncoder.js strings, no spend-ready output or authority.
use tradelayer_core::legacy_txencoder_audit::{
    source_encode_activate_unverified, source_encode_amount_unverified,
    source_encode_attestation_unverified, source_encode_bridge_unverified,
    source_encode_close_oracle_unverified, source_encode_colored_coin_unverified,
    source_encode_commit_unverified, source_encode_create_whitelist_unverified,
    source_encode_send_unverified, source_encode_smart_bind_unverified,
    source_encode_stake_fraud_relay_unverified, source_encode_token_issue_unverified,
    source_encode_transfer_unverified, source_encode_update_admin_unverified,
    source_encode_withdrawal_unverified, ClaimedClearLists, ClaimedCommit, ClaimedSend,
    ClaimedStakeFraudRelay, ClaimedTokenIssue, ClaimedTransfer, ClaimedWithdrawal,
    LegacyEncoderError, SourceEncoderCaveat, SourceInnerEnvelope, SourceRevokeArgument,
    UnverifiedSourceEncoding, ATOMS_PER_TOKEN, JS_SAFE_INTEGER, MAX_SOURCE_LIST_IDS,
};
use tradelayer_core::legacy_types_audit::{GenericEnvelopeKind, LegacyTypesError};
use tradelayer_core::legacy_txdecoder_audit::{
    source_decode_body_unverified, UnverifiedSourceBody, LegacyDecoderError,
};

fn source_case(tag: &str, long: &str, code_hash: &str) -> UnverifiedSourceEncoding {
    match tag {
        "activation-256" => source_encode_activate_unverified(&[0, 9], code_hash),
        "token-issue-multi" => source_encode_token_issue_unverified(ClaimedTokenIssue {
            initial_amount: 10,
            ticker: "TK",
            whitelists: &[7, 11],
            managed: true,
            backup_address: "backupB",
            nft: false,
            colored_coin_hybrid: false,
            procedural_type: None,
        }),
        "send-all" => source_encode_send_unverified(ClaimedSend::SendAll {
            recipient: "alice",
        }),
        "send-whole" => source_encode_send_unverified(ClaimedSend::Single {
            recipient: "alice",
            property_id: 5,
            amount_atoms: 2 * ATOMS_PER_TOKEN,
        }),
        "send-fraction" => source_encode_send_unverified(ClaimedSend::Single {
            recipient: "alice",
            property_id: 5,
            amount_atoms: ATOMS_PER_TOKEN / 2,
        }),
        "send-multi" => source_encode_send_unverified(ClaimedSend::Multi {
            property_ids: &[5, 7],
            amount_atoms: &[ATOMS_PER_TOKEN, ATOMS_PER_TOKEN / 2],
        }),
        "commit-single" => source_encode_commit_unverified(ClaimedCommit {
            property_id: 7, amount_atoms: ATOMS_PER_TOKEN,
            channel_address: "channel1", ref_vout: 4, pay_enabled: true,
            clear_lists: ClaimedClearLists::Array(&[7]), colored_output: false,
            commit_clearlist_id: Some(11),
        }),
        "commit-multi" => source_encode_commit_unverified(ClaimedCommit {
            property_id: 7, amount_atoms: ATOMS_PER_TOKEN,
            channel_address: "channel1", ref_vout: 4, pay_enabled: false,
            clear_lists: ClaimedClearLists::Array(&[7, 11]), colored_output: true,
            commit_clearlist_id: None,
        }),
        "commit-long-ref" => source_encode_commit_unverified(ClaimedCommit {
            property_id: 7, amount_atoms: ATOMS_PER_TOKEN,
            channel_address: long, ref_vout: 4, pay_enabled: false,
            clear_lists: ClaimedClearLists::Scalar(11), colored_output: true,
            commit_clearlist_id: None,
        }),
        "whitelist" => {
            source_encode_create_whitelist_unverified("backupB", "ListOne", "site", "note")
        }
        "admin" => {
            source_encode_update_admin_unverified("adminC", true, false, false, 11, true)
        }
        "attest-numeric" => source_encode_attestation_unverified(
            SourceRevokeArgument::Numeric(false), 7, "alice", "ok",
        ),
        "attest-boolean-revoke" => source_encode_attestation_unverified(
            SourceRevokeArgument::Boolean(true), 7, "alice", "ok",
        ),
        "close-ignores-id" => source_encode_close_oracle_unverified(11),
        "withdraw-boolean" => source_encode_withdrawal_unverified(ClaimedWithdrawal {
            withdraw_all: true, property_id: 7, amount_atoms: ATOMS_PER_TOKEN,
            column: 1, channel_address: "channel1", ref_vout: 10,
        }),
        "withdraw-long-ref" => source_encode_withdrawal_unverified(ClaimedWithdrawal {
            withdraw_all: false, property_id: 7, amount_atoms: ATOMS_PER_TOKEN / 2,
            column: 0, channel_address: long, ref_vout: 10,
        }),
        "transfer-direct" => source_encode_transfer_unverified(ClaimedTransfer {
            property_id: 7, amount_atoms: ATOMS_PER_TOKEN, is_column_a: true,
            destination_address: "chanB", ref_vout: 3, pay: true, pay_ref: "pay",
        }),
        "transfer-long-ref" => source_encode_transfer_unverified(ClaimedTransfer {
            property_id: 7, amount_atoms: ATOMS_PER_TOKEN / 2, is_column_a: false,
            destination_address: long, ref_vout: 3, pay: false, pay_ref: "",
        }),
        "stake-fraud-relay" => {
            source_encode_stake_fraud_relay_unverified(ClaimedStakeFraudRelay {
                action: 1, oracle_id: 11, staked_property_id: 7,
                amount_atoms: ATOMS_PER_TOKEN, accused_address: "alice",
                evidence_hash: "evidence", relay_type: 2, state_hash: "state",
                dlc_ref: "dlc", settlement_state: "settle", relay_blob: "blob",
                auto_roll: true, next_dlc_ref: "next",
            })
        }
        "colored-body" => source_encode_colored_coin_unverified(2, 7, 10, "home"),
        "bridge-body" => source_encode_bridge_unverified(7, 11, "destination"),
        "bind-body" => source_encode_smart_bind_unverified("hash", "2", "btc"),
        _ => panic!("unknown frozen encoder vector {tag}"),
    }.unwrap()
}

#[test]
fn all_22_shared_pinned_source_inner_and_generic_encoder_strings_match() {
    let text = include_str!("../fixtures/legacy_txencoder_source_vectors.tsv");
    let long = "C".repeat(43);
    let code_hash = format!("{}ff", "0".repeat(62));
    let rows = text.lines()
        .filter(|line| !line.is_empty()
            && !line.starts_with('#')
            && !line.starts_with("tag\t"))
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 22);
    for line in rows {
        let columns = line.split('\t').collect::<Vec<_>>();
        assert_eq!(columns.len(), 3);
        let observation = source_case(columns[0], &long, &code_hash);
        assert_eq!(observation.source_return(), columns[1], "{}", columns[0]);
        let wants_generic = columns[2];
        if wants_generic == "!missing" {
            assert_eq!(observation.type_id(), 4);
            assert_eq!(observation.source_generic_types_concat_unverified(),
                Err(LegacyTypesError::MissingSourceEncoder(4)));
        } else {
            let generic = observation.source_generic_types_concat_unverified().unwrap();
            assert_eq!(generic.claimed_ascii(), wants_generic, "{}", columns[0]);
            assert_eq!(generic.require_encoded_transaction_authority(),
                Err(LegacyTypesError::TrustedTransactionExecutionForbidden));
        }
        assert_eq!(observation.require_historically_accepted_transaction(),
            Err(LegacyEncoderError::HistoricalTransactionNotAuthenticated));
        assert_eq!(observation.require_authenticated_list_activation(),
            Err(LegacyEncoderError::HistoricalListAndActivationUnverified));
        assert_eq!(observation.require_signed_channel_revision(),
            Err(LegacyEncoderError::CanonicalSignedRevisionUnavailable));
        assert_eq!(observation.require_trusted_execution(),
            Err(LegacyEncoderError::TrustedEconomicExecutionForbidden));
    }
}

#[test]
fn 256_bit_hash_and_fractional_amounts_are_bounded_exactly() {
    let max_hash = "f".repeat(64);
    let observation = source_encode_activate_unverified(&[35], &max_hash).unwrap();
    assert_eq!(observation.source_return(),
        "tl035,6dp5qcb22im238nr3wvp0ic7q99w035jmy2iw7i6n43d37jtof");
    let zero = "0".repeat(64);
    assert_eq!(source_encode_activate_unverified(&[0], &zero).unwrap().source_return(),
        "tl00,0");
    for invalid in ["f", "g".repeat(64).as_str(), "0".repeat(63).as_str()] {
        assert_eq!(source_encode_activate_unverified(&[0], invalid),
            Err(LegacyEncoderError::InvalidActivationHex));
    }
    assert_eq!(source_encode_activate_unverified(&[36], &zero),
        Err(LegacyEncoderError::InvalidActivationType));
    assert_eq!(source_encode_amount_unverified(2*ATOMS_PER_TOKEN), Ok("2".into()));
    assert_eq!(source_encode_amount_unverified(ATOMS_PER_TOKEN/2), Ok("tro8w".into()));
    assert_eq!(source_encode_amount_unverified(1), Ok("1".into()));
    assert_eq!(source_encode_amount_unverified(JS_SAFE_INTEGER),
        Ok("2gosa7pa2gv".into()));
    assert_eq!(source_encode_amount_unverified(JS_SAFE_INTEGER + 1),
        Err(LegacyEncoderError::NotExactSafeJavaScriptNumber));
}

#[test]
fn source_send_scalar_multi_and_missing_tilde_remain_observations_only() {
    let fractional = source_encode_send_unverified(ClaimedSend::Single {
        recipient:"alice", property_id:5, amount_atoms:ATOMS_PER_TOKEN/2,
    }).unwrap();
    assert_eq!(fractional.source_inner_prefix_shape(),SourceInnerEnvelope::SingleTypePrefixed);
    assert!(fractional.caveats().contains(&SourceEncoderCaveat::FractionalAmountMissingTilde));
    let generic=fractional.source_generic_types_concat_unverified().unwrap();
    assert_eq!(generic.envelope_kind(),GenericEnvelopeKind::DuplicatedTypePrefix);
    let all=source_encode_send_unverified(ClaimedSend::SendAll {recipient:"alice"}).unwrap();
    assert_eq!(all.source_inner_prefix_shape(),SourceInnerEnvelope::BodyOnly);
    assert_eq!(all.source_generic_types_concat_unverified().unwrap().envelope_kind(),
        GenericEnvelopeKind::SingleOuterPrefix);
    let multi=source_encode_send_unverified(ClaimedSend::Multi {
        property_ids:&[5,7],amount_atoms:&[ATOMS_PER_TOKEN,ATOMS_PER_TOKEN/2],
    }).unwrap();
    assert_eq!(multi.source_return(),"0;;5,7;1,tro8w");
    assert!(multi.caveats().contains(&SourceEncoderCaveat::MultiSendMisparsedAsScalarByDecoder));
    assert_eq!(source_encode_send_unverified(ClaimedSend::Multi {
        property_ids:&[7],amount_atoms:&[1,2],
    }),Err(LegacyEncoderError::MismatchedMultiSendArrays));
    let overflow=vec![7u64; MAX_SOURCE_LIST_IDS+1];
    let amounts=vec![1u128; MAX_SOURCE_LIST_IDS+1];
    assert_eq!(source_encode_send_unverified(ClaimedSend::Multi {
        property_ids:&overflow,amount_atoms:&amounts,
    }),Err(LegacyEncoderError::TooManyListIds));
}

#[test]
fn commit_clearlist_and_ref_source_encodings_never_grant_channel_ownership() {
    let ref_address="Q".repeat(43);
    let direct=source_encode_commit_unverified(ClaimedCommit {
        property_id:7,amount_atoms:ATOMS_PER_TOKEN,channel_address:&ref_address,
        ref_vout:4,pay_enabled:false,clear_lists:ClaimedClearLists::Scalar(11),
        colored_output:true,commit_clearlist_id:None,
    }).unwrap();
    assert_eq!(direct.source_return(),"tl47,1njchs,ref:4,0,b,1");
    assert!(direct.caveats().contains(
        &SourceEncoderCaveat::GenericTypesEncoderMissingType4Export));
    assert!(direct.caveats().contains(
        &SourceEncoderCaveat::LongAddressSubstitutedByCallerClaimedRef));
    assert_eq!(direct.source_generic_types_concat_unverified(),
        Err(LegacyTypesError::MissingSourceEncoder(4)));
    let multiple=source_encode_commit_unverified(ClaimedCommit {
        property_id:7,amount_atoms:ATOMS_PER_TOKEN,channel_address:"channel1",
        ref_vout:0,pay_enabled:true,clear_lists:ClaimedClearLists::Array(&[7,11]),
        colored_output:false,commit_clearlist_id:Some(11),
    }).unwrap();
    assert!(multiple.source_return().contains("[7,b]"));
    assert!(multiple.caveats().contains(
        &SourceEncoderCaveat::CommitClearlistCommasBreakDecoder));
    assert_eq!(source_decode_body_unverified(4, multiple.source_return().strip_prefix("tl4").unwrap()),
        Err(LegacyDecoderError::BracketedListSplitBeforeParsing));
    assert_eq!(multiple.require_signed_channel_revision(),
        Err(LegacyEncoderError::CanonicalSignedRevisionUnavailable));
}

#[test]
fn type9_boolean_revoke_and_type15_ignored_id_are_source_bugs_not_credentials() {
    let bool_revoke=source_encode_attestation_unverified(
        SourceRevokeArgument::Boolean(true),7,"alice","ok",
    ).unwrap();
    assert_eq!(bool_revoke.source_return(),"tl9true,7,alice,ok");
    assert!(bool_revoke.caveats().contains(
        &SourceEncoderCaveat::RawBooleanRevokeDoesNotDecodeAsTrue));
    let parsed=source_decode_body_unverified(9,
        bool_revoke.source_return().strip_prefix("tl9").unwrap()).unwrap();
    assert_eq!(parsed.source_fields(),&UnverifiedSourceBody::Attestation {
        revoke:false,list_id:7,target_address:"alice".into(),metadata_upper:"OK".into(),
    });
    let numeric=source_encode_attestation_unverified(
        SourceRevokeArgument::Numeric(true),7,"alice","ok",
    ).unwrap();
    let p=source_decode_body_unverified(9,
        numeric.source_return().strip_prefix("tl9").unwrap()).unwrap();
    match p.source_fields() {
        UnverifiedSourceBody::Attestation {revoke,..} => assert!(*revoke),
        _=>panic!("expected attestation"),
    }
    assert_eq!(source_encode_close_oracle_unverified(7).unwrap().source_return(),
        source_encode_close_oracle_unverified(11).unwrap().source_return());
    let close=source_encode_close_oracle_unverified(100).unwrap();
    assert_eq!(close.source_return(),"tlf");
    assert!(close.caveats().contains(&SourceEncoderCaveat::CloseOracleIgnoresIdAndReturnsPrefixOnly));
    assert_eq!(close.source_generic_types_concat_unverified().unwrap().claimed_ascii(),"tlftlf");
}

#[test]
fn type21_decimal_amount_and_boolean_literal_fail_source_decoder_expectations() {
    let withdraw=source_encode_withdrawal_unverified(ClaimedWithdrawal {
        withdraw_all:true,property_id:7,amount_atoms:ATOMS_PER_TOKEN,
        column:1,channel_address:"channel1",ref_vout:0,
    }).unwrap();
    assert_eq!(withdraw.source_return(),"tlltrue,7,100000000,1,channel1");
    assert!(withdraw.caveats().contains(
        &SourceEncoderCaveat::WithdrawalDecimalAmountVersusBase36Decoder));
    assert!(withdraw.caveats().contains(
        &SourceEncoderCaveat::WithdrawalBooleanLiteralVersusOneFlag));
    let after_prefix=withdraw.source_return().strip_prefix("tll").unwrap();
    let parsed=source_decode_body_unverified(21,after_prefix).unwrap();
    match parsed.source_fields() {
        UnverifiedSourceBody::Withdrawal { withdraw_all,amount_atoms, .. } => {
            assert!(!withdraw_all);
            assert_eq!(*amount_atoms,2_821_109_907_456u128);
            assert_ne!(*amount_atoms,ATOMS_PER_TOKEN);
        }
        _=>panic!("expected withdrawn decoder body"),
    }
    assert_eq!(withdraw.require_trusted_execution(),
        Err(LegacyEncoderError::TrustedEconomicExecutionForbidden));
}

#[test]
fn body_only_33_34_35_encode_paths_are_not_source_decoder_acceptance() {
    for observation in [
        source_encode_colored_coin_unverified(2,7,10,"home").unwrap(),
        source_encode_bridge_unverified(7,11,"destination").unwrap(),
        source_encode_smart_bind_unverified("hash","2","btc").unwrap(),
    ] {
        assert_eq!(observation.source_inner_prefix_shape(),SourceInnerEnvelope::BodyOnly);
        assert_eq!(observation.source_generic_types_concat_unverified().unwrap()
            .envelope_kind(),GenericEnvelopeKind::SingleOuterPrefix);
        assert_eq!(observation.require_historically_accepted_transaction(),
            Err(LegacyEncoderError::HistoricalTransactionNotAuthenticated));
    }
    let bridge=source_encode_bridge_unverified(7,11,"destination").unwrap();
    assert_eq!(bridge.source_return(),"7,b,destination");
    assert_eq!(bridge.source_generic_types_concat_unverified().unwrap()
        .claimed_ascii(),"tly7,b,destination");
}

#[test]
fn invalid_claims_deny_non_ascii_oversize_and_unproven_numeric_fallback() {
    assert_eq!(source_encode_create_whitelist_unverified("bad\n","N","U","D"),
        Err(LegacyEncoderError::NonPrintableAscii));
    assert_eq!(source_encode_smart_bind_unverified("hash","☃","btc"),
        Err(LegacyEncoderError::NonPrintableAscii));
    assert_eq!(source_encode_create_whitelist_unverified("backup","N",&"x".repeat(4097),"D"),
        Err(LegacyEncoderError::SourceReturnTooLong));
    assert_eq!(source_encode_transfer_unverified(ClaimedTransfer {
        property_id:7,amount_atoms:JS_SAFE_INTEGER+1,is_column_a:true,
        destination_address:"destination",ref_vout:0,pay:false,pay_ref:"",
    }),Err(LegacyEncoderError::NotExactSafeJavaScriptNumber));
    assert_eq!(source_encode_withdrawal_unverified(ClaimedWithdrawal {
        withdraw_all:true,property_id:7,amount_atoms:100,column:2,
        channel_address:"channel1",ref_vout:0,
    }),Err(LegacyEncoderError::MissingOrMalformedField));
    assert_eq!(source_encode_commit_unverified(ClaimedCommit {
        property_id:7,amount_atoms:1,channel_address:"",
        ref_vout:0,pay_enabled:false,clear_lists:ClaimedClearLists::NotSet,
        colored_output:false,commit_clearlist_id:None,
    }),Err(LegacyEncoderError::MissingOrMalformedField));
}
