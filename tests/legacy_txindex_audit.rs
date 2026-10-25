//! File8 txIndex.js source compatibility is distinct from faster Rust scan.
use tradelayer_core::legacy_txindex_audit::{
    source_extract_first_nulldata_unverified,
    source_index_row_key_unverified, source_project_refs_0_and_1_unverified,
    source_updates_max_height_in_loop_on_tl_only,
    require_authenticated_reference_output, require_historical_index_consensus,
    ClaimedOutput,LegacyIndexError,
};
use tradelayer_core::legacy_types_audit::{SourceDecodeCaveat,SourceDecodeOutcome};

fn vout<'a>(index:u32,nulldata:bool,hex:&'a str,address:&'a str)->ClaimedOutput<'a>{
    ClaimedOutput {
        vout:index,is_nulldata:nulldata,script_hex:hex,
        address,satoshis:500+u64::from(index)
    }
}
fn script(data:&str)->String{
    let bytes=data.bytes().map(|b| format!("{b:02x}")).collect::<String>();
    format!("6a{:02x}746c{bytes}",data.len()+2)
}

#[test]
fn exact_source_marker_from_first_nulldata_not_second() {
    let good=script("20;alice;5;2");
    let normal=source_extract_first_nulldata_unverified(&[
        vout(0,false,&good,"miner"),vout(1,true,&good,"burn")
    ]).unwrap().unwrap();
    assert_eq!(normal.payload_after_marker(),"20;alice;5;2");
    assert_eq!(normal.selected_vout(),1);
    assert_eq!(normal.source_nibble_position(),4);
    // TxIndex passes only 3 args to a 9-arg decoder. Its decodedParams
    // becomes a JS Error object in the ordinary after-marker payload.
    let e=normal.legacy_decoder_entry();
    assert_eq!(e.outcome(),SourceDecodeOutcome::ReturnedErrorObjectInvalidMarker);
    assert_eq!(e.caveats(),&[SourceDecodeCaveat::TxIndexMisorderedCallSignature]);
    assert_eq!(normal.require_historical_transaction_acceptance(),
        Err(LegacyIndexError::NoAcceptedConsensusOrSignedRevision));
    // First nulldata lacking marker aborts without examining a later one.
    assert_eq!(source_extract_first_nulldata_unverified(&[
        vout(0,true,"6a0568656c6c6f","none"),
        vout(1,true,&good,"candidate")
    ]),Ok(None));
    assert_eq!(source_extract_first_nulldata_unverified(&[
        vout(0,false,&good,"not-null-data")
    ]),Ok(None));
}
#[test]
fn unusual_nibble_offsets_odd_hex_and_empty_payload_are_source_only() {
    // Marker at nibble offset 2 is not canonical Bitcoin OP_RETURN push,
    // yet legacy txIndex scans for "746c" within first six hex chars.
    let noncanonical=source_extract_first_nulldata_unverified(&[
        vout(0,true,"6a746c32303b616c696365","x")
    ]).unwrap().unwrap();
    assert_eq!(noncanonical.source_nibble_position(),2);
    assert_eq!(noncanonical.payload_after_marker(),"20;alice");
    // Buffer.from(hex) ignores final orphan nibble, unlike Rust strict hex.
    let odd=source_extract_first_nulldata_unverified(&[
        vout(0,true,"6a02746c3230f","x")
    ]).unwrap().unwrap();
    assert_eq!(odd.payload_after_marker(),"20");
    let empty=source_extract_first_nulldata_unverified(&[
        vout(0,true,"6a02746c","x")
    ]).unwrap().unwrap();
    assert_eq!(empty.payload_after_marker(),"");
    assert_eq!(empty.legacy_decoder_entry().outcome(),
        SourceDecodeOutcome::ReturnedErrorObjectInvalidMarker);
    assert_eq!(source_extract_first_nulldata_unverified(&[
        vout(0,true,"6a02746c00","x")
    ]),Err(LegacyIndexError::NonPrintableCarrier));
    assert_eq!(source_extract_first_nulldata_unverified(&[
        vout(0,true,"6a02746cff","x")
    ]),Err(LegacyIndexError::NonPrintableCarrier));
    assert_eq!(source_extract_first_nulldata_unverified(&[
        vout(0,true,"746C2020","x")
    ]),Ok(None)); // case-sensitive lowercase marker
    assert_eq!(source_extract_first_nulldata_unverified(&[
        vout(0,true,&"00".repeat(4097),"x")
    ]),Err(LegacyIndexError::OversizedSourceHex));
}
#[test]
fn source_reference_filter_drops_original_vout_and_data_is_untrusted() {
    let outputs=[
        vout(2,false,"","change"),
        vout(1,false,"","other"),
        vout(0,true,"","empty"),
        vout(3,false,"","later"),
    ];
    let observed=source_project_refs_0_and_1_unverified(&outputs);
    assert_eq!(observed.len(),2);
    assert_eq!(observed[0].address,"other");
    assert_eq!(observed[0].satoshis,501);
    assert_eq!(observed[1].address,"empty");
    assert_eq!(observed[1].satoshis,500);
    // txIndex discards the vout numbers when mapping references, so
    // output-array position zero is not synonymous with vout zero.
    assert_eq!(require_authenticated_reference_output(),
        Err(LegacyIndexError::UnverifiedFirstInputAndReferences));
}
#[test]
fn source_doc_keys_alias_forks_and_maxheight_untrusted() {
    let a=source_index_row_key_unverified(501,"abcd").unwrap();
    let b=source_index_row_key_unverified(501,"abcd").unwrap();
    assert_eq!(a,b);
    assert_eq!(a,"tx-501-abcd");
    assert_eq!(source_index_row_key_unverified(501,""),
        Err(LegacyIndexError::EmptySourceTxid));
    assert!(source_updates_max_height_in_loop_on_tl_only());
    assert_eq!(require_historical_index_consensus(),
        Err(LegacyIndexError::UnverifiedSourceOrdering));
}
