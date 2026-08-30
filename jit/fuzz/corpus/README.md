# JIT fuzz corpus format

Every persistent seed starts with ASCII
`QJSJFZ01:05d5c0867521c077:`, combining the corpus version and exact opcode
fingerprint, then target-specific payload bytes.
Targets impose bounded input/slot/program fuel and treat every decode, verifier,
frame-state, lowering, and relocation rejection as an ordinary result. A crash,
panic, invalid native publication, or interpreter/JIT observation mismatch is a
failure. Corpus migrations must change the eight-byte version header.
