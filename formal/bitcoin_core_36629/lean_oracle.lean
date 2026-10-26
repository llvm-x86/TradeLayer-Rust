import OrderedEncoding

open BitcoinCore36629.Encoding

def referenceValues : List Nat :=
  [0, 1, 2, 126, 127, 128, 129, 255, 256, 16510, 16511, 16512,
   16513, 2000000, 2113662, 2113663, 2113664, 2113665,
   270549118, 270549119, 270549120, 270549121,
   4000000000, 4294967294, 4294967295, max5 - 1, max5] ++
  (List.range 4096).map (fun i => (i * 2654435761 + 0x12345678) % (max5 + 1))

def main : IO Unit := do
  for n in referenceValues do
    IO.println s!"{n} {String.intercalate "," ((encode n).map toString)}"
