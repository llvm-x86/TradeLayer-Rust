//! Stage 23C: authenticate an exported Litecoin source trace from raw bytes.
//! Prints script-derived actors and references per candidate, or REJECTED
//! per block. Read-only: the output grants no replay authority.
use std::{env, fs, process};
use tradelayer_core::litecoin_evidence::{
    authenticate_block, AttestedHeaderChain, CandidateEvidence, InclusionProof, LitecoinChain,
    ParentEvidence, RawHeader, ScriptAddress,
};
use tradelayer_core::read_only::decode_hex;

fn fail(message: &str) -> ! {
    eprintln!("tradelayer-ltc-authenticate: {message}");
    process::exit(2);
}

fn raw_hash(display: &str) -> [u8; 32] {
    let mut bytes: [u8; 32] = decode_hex(display)
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or_else(|| fail("invalid 32-byte hash"));
    bytes.reverse();
    bytes
}

fn bytes(hex: &str) -> Vec<u8> {
    decode_hex(hex).unwrap_or_else(|_| fail("invalid hex"))
}

fn number<T: std::str::FromStr>(text: &str) -> T {
    text.parse().unwrap_or_else(|_| fail("invalid number"))
}

fn branch(csv: &str) -> Vec<[u8; 32]> {
    if csv == "-" {
        Vec::new()
    } else {
        csv.split(',').map(raw_hash).collect()
    }
}

fn header(hex: &str) -> RawHeader {
    RawHeader::from_bytes(&bytes(hex)).unwrap_or_else(|_| fail("invalid header"))
}

fn address(a: &ScriptAddress) -> String {
    match a {
        ScriptAddress::Single(s) => format!("single:{s}"),
        ScriptAddress::Ambiguous(_) => "ambiguous".into(),
        ScriptAddress::NoAddress(reason) => format!("none:{reason}"),
    }
}

fn short(a: &ScriptAddress) -> String {
    match a {
        ScriptAddress::Single(s) => s.clone(),
        ScriptAddress::Ambiguous(_) => "ambiguous".into(),
        ScriptAddress::NoAddress(_) => "none".into(),
    }
}

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| fail("usage: tradelayer-ltc-authenticate <export.tsv>"));
    let text = fs::read_to_string(path).unwrap_or_else(|_| fail("cannot read export"));
    let mut lines = text.lines();
    let meta: Vec<&str> = lines
        .next()
        .unwrap_or_else(|| fail("empty export"))
        .split('\t')
        .collect();
    if meta.len() != 4 || meta[0] != "# litecoin-evidence-export-v1" {
        fail("not a litecoin-evidence-export-v1 file");
    }
    let chain = match meta[1] {
        "main" => LitecoinChain::Mainnet,
        "test" => LitecoinChain::Testnet,
        _ => fail("unsupported network"),
    };
    let start: u64 = number(meta[2]);
    let previous = (meta[3] != "-").then(|| raw_hash(meta[3]));
    let mut headers = Vec::new();
    let mut candidates: Vec<(u64, CandidateEvidence)> = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        match f.first().copied() {
            Some("H") if f.len() == 3 => {
                if number::<u64>(f[1]) != start + headers.len() as u64 {
                    fail("headers out of order");
                }
                headers.push(header(f[2]));
            }
            Some("C") if f.len() == 12 => candidates.push((
                number(f[1]),
                CandidateEvidence {
                    raw_tx: bytes(f[5]),
                    inclusion: InclusionProof {
                        header: RawHeader::from_fields(0, [0; 32], [0; 32], 0, 0, 0),
                        branch: branch(f[4]),
                        position: number(f[2]),
                        tx_count: number(f[3]),
                    },
                    parent: Some(ParentEvidence {
                        raw_tx: bytes(f[11]),
                        height: number(f[6]),
                        inclusion: InclusionProof {
                            header: header(f[10]),
                            branch: branch(f[9]),
                            position: number(f[7]),
                            tx_count: number(f[8]),
                        },
                    }),
                },
            )),
            _ => fail("malformed export line"),
        }
    }
    let attested = AttestedHeaderChain::from_headers(chain, start, previous, headers)
        .unwrap_or_else(|err| fail(&format!("header segment rejected: {err}")));
    println!("# txid\theight\tposition\tparent_membership\tsender\tactor_resolved\tvout_0_1\tlast_non_nulldata");
    let mut index = 0;
    while index < candidates.len() {
        let height = candidates[index].0;
        let mut block = Vec::new();
        while index < candidates.len() && candidates[index].0 == height {
            let mut c = candidates[index].1.clone();
            // The candidate's header is the attested one at its height; a
            // mismatch is caught by Merkle verification against that root.
            c.inclusion.header = attested
                .header_at(height)
                .cloned()
                .unwrap_or_else(|| RawHeader::from_fields(0, [0; 32], [0; 32], 0, 0, 0));
            block.push(c);
            index += 1;
        }
        match authenticate_block(&attested, height, &block) {
            Ok(observations) => {
                for o in observations {
                    let mut txid = o.txid();
                    txid.reverse();
                    let refs: Vec<String> = o
                        .reference_vout_0_1()
                        .iter()
                        .map(|r| format!("{}:{}", r.vout, short(&r.address)))
                        .collect();
                    println!(
                        "{}\t{}\t{}\t{:?}\t{}\t{}\t{}\t{}",
                        txid.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                        o.height(),
                        o.position(),
                        o.parent_membership(),
                        address(o.sender_address()),
                        o.actor_resolved(),
                        if refs.is_empty() {
                            "-".into()
                        } else {
                            refs.join(",")
                        },
                        o.last_non_nulldata().map_or("-".into(), |r| format!(
                            "{}:{}",
                            r.vout,
                            short(&r.address)
                        )),
                    );
                }
            }
            Err(err) => println!("REJECTED\t{height}\t{err}"),
        }
    }
}
