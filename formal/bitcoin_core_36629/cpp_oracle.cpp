// Source oracle: compile and execute EXACT pinned upstream
// src/index/ordered_varint.h against a narrow byte-stream adapter.
// The formatter functions themselves remain unchanged.
#include <index/ordered_varint.h>
#include <cstdint>
#include <exception>
#include <iostream>
#include <limits>
#include <stdexcept>
#include <vector>
struct ByteStream {
    std::vector<uint8_t> bytes;
    std::size_t cursor{0};
    void put(uint8_t b) { bytes.push_back(b); }
    uint8_t get() {
        if (cursor >= bytes.size()) throw std::ios_base::failure("truncated");
        return bytes[cursor++];
    }
};

template <typename T>
bool read_rejected(std::vector<uint8_t> bytes) {
    ByteStream stream{std::move(bytes)};
    T out{};
    try {
        OrderedVarIntFormatter{}.Unser(stream, out);
    } catch (std::ios_base::failure const&) {
        return true;
    }
    return false;
}

int main() {
    // Extra edge checks run against the pinned C++ implementation itself.
    for (uint16_t prefix = 0xf8; prefix <= 0xff; ++prefix) {
        if (!read_rejected<uint64_t>({static_cast<uint8_t>(prefix), 0, 0, 0, 0})) return 2;
    }
    for (auto bytes : {std::vector<uint8_t>{0x80},
                       std::vector<uint8_t>{0xc0, 0},
                       std::vector<uint8_t>{0xe0, 0, 0},
                       std::vector<uint8_t>{0xf0, 0, 0, 0}}) {
        if (!read_rejected<uint64_t>(std::move(bytes))) return 3;
    }
    {
        ByteStream stream;
        try {
            OrderedVarIntFormatter{}.Ser(stream, uint64_t{34630287488ULL});
            return 4;
        } catch (std::ios_base::failure const&) {}
    }
    {
        ByteStream stream;
        OrderedVarIntFormatter{}.Ser(stream, uint64_t{4294967296ULL});
        if (!read_rejected<uint32_t>(stream.bytes)) return 5;
    }

    uint64_t value{};
    while (std::cin >> value) {
        ByteStream s;
        OrderedVarIntFormatter{}.Ser(s, value);
        uint64_t decoded{};
        OrderedVarIntFormatter{}.Unser(s, decoded);
        if (decoded != value || s.cursor != s.bytes.size()) return 6;
        std::cout << value << " ";
        for (std::size_t i=0; i<s.bytes.size(); ++i) {
            if (i) std::cout << ",";
            std::cout << static_cast<unsigned>(s.bytes[i]);
        }
        std::cout << "\n";
    }
    if (!std::cin.eof()) return 7;
    return 0;
}
