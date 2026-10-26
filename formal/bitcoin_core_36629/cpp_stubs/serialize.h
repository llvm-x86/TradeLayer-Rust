// Narrow test adapter for the unchanged Bitcoin Core header.
// This is NOT Bitcoin Core's serialize.h; it implements only the
// ser_writedata8/ser_readdata8 stream operations exercised by the formatter.
#ifndef BITCOIN_LEAN_TEST_SERIALIZE_STUB_H
#define BITCOIN_LEAN_TEST_SERIALIZE_STUB_H
#include <cstdint>
template <typename Stream>
void ser_writedata8(Stream& s, uint8_t value) { s.put(value); }
template <typename Stream>
uint8_t ser_readdata8(Stream& s) { return s.get(); }
#endif
