#!/usr/bin/env node
"use strict";

/*
 * Independent pinned-source expression characterization for
 * tradelayer.js@dca0888b src/validity.js:83-94,113-162,883-1099.
 * The helper expressions are intentionally NOT imported from the legacy
 * DB/RPC/bitcore stack; this runs without network or signing permissions.
 * Successful matches mean "if externally supplied facts hold", not an
 * actual historical accepted transaction or permission to mutate Ledger.
 */
const assert = require("node:assert/strict");

function isActivated(_block, _txid, _txType) {
    // The pinned historical function has a commented block-height test,
    // followed by an unconditional return true.
    return true;
}

function type0(sender, admin, raw) {
    const p = {valid:true, reason:"", txTypesToActivate:raw};
    let txTypes = [];
    if (Array.isArray(p.txTypesToActivate)) {
        txTypes = p.txTypesToActivate
            .map(tx=>Number(tx))
            .filter(tx=>!isNaN(tx));
    } else if (p.txTypeToActivate !== undefined &&
               p.txTypesToActivate !== null) {
        const num = Number(p.txTypesToActivate);
        if (!isNaN(num)) txTypes = [num];
    }
    if (!txTypes.length) {
        p.valid = false; p.reason = "Tx Type contains non-integer or invalid values";
        return p;
    }
    if (sender !== admin) {
        p.valid = false; p.reason = "Not sent from admin address";
        return p;
    }
    if (txTypes.some(x=>x > 35 || x < 0)) {
        p.valid = false; p.reason = "Tx Type out of bounds";
        return p;
    }
    p.txTypesToActivate = txTypes;
    return p;
}

function type7(active, backupAddress, name, validLibrary, wrapper) {
    const p = {backupAddress, name, valid:true, reason:""};
    if (!active) { p.valid = false; p.reason += "Tx type not yet activated "; }
    if (!isActivated(1, "fake-txid", 7)) { p.valid=false; }
    if (!(p.backupAddress && typeof p.backupAddress === "string")) {
        p.valid=false;p.reason+="Invalid backup address; ";
    }
    if (!validLibrary) {
        const valid = wrapper;
        if (!valid.isValid && !valid.isvalid) {
            p.valid=false; p.reason="Destination address is not validly formed.";
        }
    }
    if (!(typeof p.name === "string")) {
        p.valid=false;p.reason+="Invalid name; ";
    }
    return p;
}

function type8(ctx) {
    const p = {valid:true,reason:""};
    if (!ctx.active) {p.valid=false;p.reason+="Tx type not yet activated; ";}
    if (!isActivated(1,"fake-txid",8)) {p.valid=false;}
    if (!(typeof ctx.newAddress === "string")) {
        p.valid=false;p.reason+="Invalid new address; ";
    }
    const n=[Boolean(ctx.whitelist),Boolean(ctx.oracle),Boolean(ctx.token)]
        .filter(Boolean).length;
    if (n !== 1) {
        p.valid=false;p.reason+="Exactly one admin target (whitelist/oracle/token) must be selected; ";
    }
    if (ctx.whitelist) {
        // ClearList.getList(id) queries the attestations collection,
        // not the canonical created clearlists collection.
        const row = ctx.attestations.get(ctx.id) || null;
        if (!row) {
            p.valid=false;p.reason+="Whitelist not found; ";
        } else if (row.adminAddress !== ctx.sender &&
                   row.backupAddress !== ctx.sender) {
            p.valid=false;p.reason+="Sender is not the admin of the whitelist; ";
        }
    }
    if (!ctx.validLibrary) {
        // Unlike Type7: the source checks ONLY lowercase isvalid here!
        if (!ctx.wrapper.isvalid) {
            p.valid=false;p.reason="Destination address is not validly formed.";
        }
    }
    // Oracle/token targets depend on unmodeled external managers and are
    // intentionally excluded from positive "source valid" assertions.
    return p;
}

function type9(ctx) {
    const p={valid:true,reason:"", targetAddress:ctx.target, id:ctx.id};
    if (!ctx.active) {p.valid=false;p.reason+="Tx type not yet activated; ";}
    if (!isActivated(1,"fake-txid",9)) {p.valid=false;}
    if (ctx.refRequested) {
        const output = ctx.outputs?.find(x=>x.vout === Number(ctx.ref));
        if (output && output.address) p.targetAddress = output.address;
        else {p.valid=false;p.reason+="Could not resolve ref; ";}
    }
    if (typeof p.targetAddress !== "string" || p.targetAddress === "") {
        p.valid=false;p.reason+="Invalid target address; ";
    }
    const zero = Number(p.id) === 0;
    const ban = zero && String(p.targetAddress || "").trim().toUpperCase() === "BANLIST";
    if (!zero) {
        const list=ctx.clearlists.get(p.id);
        if (!list) {
            p.valid=false;p.reason+="Clearlist not found; ";
        } else if (ctx.sender !== (list.adminAddress || list.admin)) {
            p.valid=false;p.reason+="Sender not authorized; ";
        }
    }
    if (zero && ban) {
        if (ctx.sender !== ctx.protocolAdmin) {
            p.valid=false;p.reason+="Only protocol admin can update global banlist; ";
        }
        const countries=String(ctx.metadata || "").split(/[,\s;|]+/)
            .map(s=>s.trim().toUpperCase()).filter(Boolean);
        if (!countries.length || countries.find(c=>!/^[A-Z]{2}$/.test(c))) {
            p.valid=false;p.reason+="Banlist update requires 2-letter country codes; ";
        }
        p.countries = countries;
    } else if (zero && ctx.sender !== p.targetAddress) {
        p.valid=false;p.reason+="Sender and target address must be the same; ";
    }
    if (zero && !ban && (!ctx.metadata ||
        !/^[A-Za-z]{2}$/.test(String(ctx.metadata).trim()))) {
        p.valid=false;p.reason+="Self-cert requires 2-letter country code; ";
    }
    if (ctx.revoke && !ctx.activeDirectAttestation) {
        p.valid=false;p.reason+="Target Address has no attestation to revoke ";
    }
    return p;
}

function main() {
    for(const type of [0,1,7,8,9,19,35,99]) {
        assert.equal(isActivated(0, "not-a-transaction", type), true);
    }
    assert.equal(type0("root","root",[NaN,1.5,0,35]).valid, true);
    assert.deepEqual(type0("root","root",[NaN,1.5,0,35]).txTypesToActivate,[1.5,0,35]);
    assert.equal(parseInt(1.5),1); // Later Activation.activate() truncates.
    assert.equal(type0("root","root",[NaN]).valid, false);
    assert.equal(type0("other","root",[1]).valid, false);
    assert.equal(type0("root","root",[36]).valid, false);
    assert.equal(type0("root","root",[Infinity]).valid, false);
    assert.equal(type0("root","root",[]).valid, false);

    // Exhaustive wrapper booleans: type7 camel/lower, type8 lower ONLY.
    for (const lib of [false,true]) {
        for(const camel of [false,true]) {
            for (const lower of [false,true]) {
                const wrapper={isValid:camel,isvalid:lower};
                const seven=type7(true,"backupB","",lib,wrapper);
                assert.equal(seven.valid, lib || camel || lower);
                const eight=type8({
                    active:true,sender:"adminA",newAddress:"adminC",
                    whitelist:true,oracle:false,token:false,id:1,
                    attestations:new Map([[1,{adminAddress:"adminA"}]]),
                    validLibrary:lib,wrapper
                });
                assert.equal(eight.valid,lib || lower);
            }
        }
    }
    assert.equal(type7(false,"backupB","Name",true,{}).valid,false);
    assert.equal(type7(true,"","Name",true,{}).valid,false);
    assert.equal(type7(true,"backupB",undefined,true,{}).valid,false);

    const registry=new Map([[1,{admin:"adminA",backup:"backupB"}]]);
    // Created source row in clearlists does not appear in getList lookup!
    const empty=new Map();
    const base={
        active:true,sender:"adminA",newAddress:"adminC",
        whitelist:true,oracle:false,token:false,id:1,
        clearlists:registry,attestations:empty,validLibrary:true,
        wrapper:{isValid:false,isvalid:false}
    };
    assert.equal(type8(base).valid,false);
    assert.equal(type8({...base,attestations:new Map([[1,{admin:"adminA"}]])}).valid,false);
    assert.equal(type8({...base,attestations:new Map([[1,{adminAddress:"adminA"}]])}).valid,true);
    assert.equal(type8({...base,attestations:new Map([[1,{backupAddress:"backupB"}]]),sender:"backupB"}).valid,true);
    assert.equal(type8({...base,whitelist:false}).valid,false);
    assert.equal(type8({...base,token:true}).valid,false);
    assert.equal(type8({...base,active:false}).valid,false);

    const common={
        active:true, sender:"adminA",id:1,target:"alice",metadata:"anything",
        clearlists:registry,protocolAdmin:"root",refRequested:false,
        revoke:false,activeDirectAttestation:false,
    };
    assert.equal(type9(common).valid,true);
    assert.equal(type9({...common,sender:"backupB"}).valid,false);
    assert.equal(type9({...common,clearlists:empty}).valid,false);
    assert.equal(type9({...common,active:false}).valid,false);
    assert.equal(type9({...common,revoke:true}).valid,false);
    assert.equal(type9({...common,revoke:true,activeDirectAttestation:true}).valid,true);
    assert.equal(type9({...common,refRequested:true,ref:1,
        outputs:[{vout:1,address:"resolved"}],target:""}).targetAddress,"resolved");
    assert.equal(type9({...common,refRequested:true,ref:2,outputs:[],target:""}).valid,false);

    const cert={...common,id:0,sender:"alice",target:"alice",metadata:" us "};
    assert.equal(type9(cert).valid,true);
    assert.equal(type9({...cert,target:"bob"}).valid,false);
    assert.equal(type9({...cert,metadata:"USA"}).valid,false);
    assert.equal(type9({...cert,metadata:"u9"}).valid,false);
    const ban={...cert,sender:"root",protocolAdmin:"root",target:" banlist ",metadata:"us;KP | nz, gb"};
    assert.equal(type9(ban).valid,true);
    assert.deepEqual(type9(ban).countries,["US","KP","NZ","GB"]);
    assert.equal(type9({...ban,sender:"alice"}).valid,false);
    assert.equal(type9({...ban,metadata:"US;USA"}).valid,false);
    assert.equal(type9({...ban,metadata:""}).valid,false);

    console.log("PASS: Type0 array Number/coercion/fractional activation source path");
    console.log("PASS: isActivated unconditional vs separate 7/8/9 active flag");
    console.log("PASS: 8-case Type7 camel/lower vs Type8 LOWERCASE-only address wrapper");
    console.log("PASS: Type8 getList attestations collection and primary/backup source field mismatch");
    console.log("PASS: Type9 direct-list primary, ref override, country and BANLIST and revoke checks");
    console.log("BOUNDARY: source-matching claims are not authenticated accepted transactions");
}
main();
