//! Read-only JSON-RPC 2.0 server over one restored TLJP checkpoint.
//! Shapes are DRAFT; see docs/RPC_READONLY.md.
//!
//! Usage:
//! tradelayer-rpcd --checkpoint <file> --network <bitcoin|litecoin|dogecoin>
//!    --admin <account> --permitted <id[,id...]> [--bind <addr:port>]
//!    [--allow-nonloopback]
//!
//! Network, admin and permitted properties are deployment policy. They are
//! checked against the checkpoint, never read from it.
use std::collections::BTreeSet;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;

use tradelayer_core::joint::JointProtocolState;
use tradelayer_core::ledger::{AccountId, PropertyId};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::rpc_server::{check_bind_policy, serve};

const USAGE: &str = "usage: tradelayer-rpcd --checkpoint <file> \
--network <bitcoin|litecoin|dogecoin> --admin <account> \
--permitted <id[,id...]> [--bind <addr:port>] [--allow-nonloopback]";
const DEFAULT_BIND: &str = "127.0.0.1:7360";

struct Config {
    checkpoint: PathBuf,
    network: TradeLayerMainnet,
    admin: AccountId,
    permitted: BTreeSet<PropertyId>,
    bind: SocketAddr,
    allow_nonloopback: bool,
}

fn parse_network(text: &str) -> Result<TradeLayerMainnet, String> {
    match text {
        "bitcoin" => Ok(TradeLayerMainnet::Bitcoin),
        "litecoin" => Ok(TradeLayerMainnet::Litecoin),
        "dogecoin" => Ok(TradeLayerMainnet::Dogecoin),
        _ => Err(format!("unknown network: {text}")),
    }
}

fn parse_permitted(text: &str) -> Result<BTreeSet<PropertyId>, String> {
    let mut ids = BTreeSet::new();
    if text.is_empty() {
        return Ok(ids);
    }
    for part in text.split(',') {
        let number: u32 = part
            .parse()
            .map_err(|_| format!("invalid property id: {part}"))?;
        let id =
            PropertyId::try_from(number).map_err(|_| format!("invalid property id: {part}"))?;
        ids.insert(id);
    }
    Ok(ids)
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Config, String> {
    let mut checkpoint = None;
    let mut network = None;
    let mut admin = None;
    let mut permitted = None;
    let mut bind = DEFAULT_BIND.to_owned();
    let mut allow_nonloopback = false;
    let mut args = args;
    while let Some(flag) = args.next() {
        if flag == "--allow-nonloopback" {
            allow_nonloopback = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--checkpoint" => checkpoint = Some(PathBuf::from(value)),
            "--network" => network = Some(parse_network(&value)?),
            "--admin" => {
                admin = Some(
                    AccountId::try_from(value.as_str())
                        .map_err(|_| format!("invalid admin account: {value}"))?,
                );
            }
            "--permitted" => permitted = Some(parse_permitted(&value)?),
            "--bind" => bind = value,
            _ => return Err(format!("unknown flag: {flag}")),
        }
    }
    let bind: SocketAddr = bind
        .parse()
        .map_err(|_| format!("invalid bind address: {bind}"))?;
    Ok(Config {
        checkpoint: checkpoint.ok_or("--checkpoint is required")?,
        network: network.ok_or("--network is required")?,
        admin: admin.ok_or("--admin is required")?,
        permitted: permitted.ok_or("--permitted is required (may be empty)")?,
        bind,
        allow_nonloopback,
    })
}

fn run() -> Result<(), (i32, String)> {
    let config = parse_args(std::env::args().skip(1)).map_err(|msg| (2, msg))?;
    check_bind_policy(config.bind, config.allow_nonloopback).map_err(|err| (2, err.to_string()))?;
    let state = JointProtocolState::from_snapshot_file(
        &config.checkpoint,
        config.network,
        &config.admin,
        &config.permitted,
    )
    .map_err(|err| (1, format!("checkpoint rejected: {err}")))?;
    let listener = TcpListener::bind(config.bind)
        .map_err(|err| (1, format!("bind {}: {err}", config.bind)))?;
    let local = listener
        .local_addr()
        .map_err(|err| (1, format!("local address: {err}")))?;
    eprintln!("tradelayer-rpcd: read-only, listening on {local}");
    serve(listener, Arc::new(state)).map_err(|err| (1, format!("serve: {err}")))
}

fn main() {
    if let Err((code, message)) = run() {
        eprintln!("tradelayer-rpcd: {message}");
        if code == 2 {
            eprintln!("{USAGE}");
        }
        std::process::exit(code);
    }
}
