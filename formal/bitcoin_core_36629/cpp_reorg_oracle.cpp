// Source-shaped execution oracle for the merger in
// bitcoin/bitcoin#36629, pinned at cf9b981fcc7ae52c7864beae35df5d60b5c322b4.
// This harness compiles a DELIBERATELY SMALL C++ extraction of the source's
// std::map<std::pair<height,tx_offset>,...>::try_emplace/append behavior.
// It is not the original RPC handler or proof that scheduler interleaving
// is reachable in the full Bitcoin Core node.
#include <cassert>
#include <cstddef>
#include <iostream>
#include <map>
#include <stdexcept>
#include <tuple>
#include <utility>
#include <vector>

struct Hit {
    unsigned block_hash;
    unsigned height;
    unsigned tx_offset;
    unsigned txid;
    std::vector<unsigned> outputs;
    std::size_t vout_count;
};

bool output_bounds_ok(const Hit& hit) {
    for (auto n : hit.outputs) {
        if (n >= hit.vout_count) return false;
    }
    return true;
}

bool source_shape_at_throws(const Hit& hit) {
    try {
        for (auto n : hit.outputs) {
            // Mirrors decoded_outputs.at(output.index) at verbosity >= 1.
            (void)std::vector<int>(hit.vout_count).at(n);
        }
    } catch (const std::out_of_range&) {
        return true;
    }
    return false;
}

int main() {
    const Hit old_hit{101, 500, 81, 1001, {0}, 1};
    const Hit new_hit{202, 500, 81, 2002, {7}, 8};

    // Two independent Find calls each see a consistent, synced index.
    const bool first_index_ready = true;
    const bool second_index_ready = true;

    // Mirror the step-5 by_position map key and try_emplace behavior.
    // Its retained transaction starts with an empty matched-output vector.
    std::map<std::pair<unsigned, unsigned>, Hit> by_position;
    for (const Hit& hit : {old_hit, new_hit}) {
        Hit blank{hit.block_hash, hit.height, hit.tx_offset, hit.txid, {},
                  hit.vout_count};
        auto [it, inserted] = by_position.try_emplace(
            std::pair{hit.height, hit.tx_offset}, blank);
        (void)inserted;
        for (unsigned n : hit.outputs) it->second.outputs.push_back(n);
    }

    // Proposed fork-aware alternative retains both transaction identities.
    std::map<std::tuple<unsigned, unsigned, unsigned>, Hit> by_block_position;
    for (const Hit& hit : {old_hit, new_hit}) {
        Hit blank{hit.block_hash, hit.height, hit.tx_offset, hit.txid, {},
                  hit.vout_count};
        auto [it, inserted] = by_block_position.try_emplace(
            std::tuple{hit.block_hash, hit.height, hit.tx_offset}, blank);
        (void)inserted;
        for (unsigned n : hit.outputs) it->second.outputs.push_back(n);
    }

    // A -> B -> A returns to the same pointer; a monotone generation differs.
    const unsigned initial_tip = 101, final_tip = 101;
    const unsigned initial_generation = 7, final_generation = 9;
    const bool final_tip_gate_accepts = initial_tip == final_tip;
    const bool epoch_gate_accepts = initial_generation == final_generation;

    assert(first_index_ready && second_index_ready);
    assert(by_position.size() == 1);
    assert(by_position.begin()->second.txid == old_hit.txid);
    assert(!output_bounds_ok(by_position.begin()->second));
    assert(source_shape_at_throws(by_position.begin()->second));
    assert(by_block_position.size() == 2);
    assert(output_bounds_ok(by_block_position.begin()->second));
    assert(output_bounds_ok(by_block_position.rbegin()->second));
    assert(final_tip_gate_accepts && !epoch_gate_accepts);

    std::cout << "first_query_synced=1\n"
              << "second_query_synced=1\n"
              << "final_tip_gate_accepts=1\n"
              << "epoch_gate_accepts=0\n"
              << "height_offset_groups=1\n"
              << "height_offset_bounds_ok=0\n"
              << "verbose_at_throws=1\n"
              << "fork_aware_groups=2\n"
              << "fork_aware_bounds_ok=1\n";
    return 0;
}
