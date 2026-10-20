# Contributing

Choose one observable behavior and describe the expected result. Keep changes
focused and add meaningful tests for behavior that could regress.

## Build and check

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
python3 -m unittest discover -s packaging/tests -v
```

## Evidence and contributions

Protocol changes need a clear behavioral explanation and tests. Hardware reports
should identify the exact model, firmware, OS, interface and report direction.
Label synthetic vectors and simulator results.
Do not infer working device support from recognition or parsing alone.

Preserve unknown values, keep parsing separate from I/O, and never replay a
persistent write silently. Writes require explicit UI intent, a fresh destination
and readback checks; hardware support claims require actual hardware evidence.

AI-assisted contributions are welcome. Disclose material AI assistance in the
pull request and describe what you personally verified. You are responsible for
reviewing changes and for the right to contribute them. No proprietary source,
decompiler output, original firmware, copied UI artwork or private presets.

Small fixes need a concise explanation and appropriate checks. Hardware reports
need the exact device and firmware and should use the acceptance workbench.
Review all attachments for personal information before sharing.

Contributions use the project's LICENSE unless a file has its own applicable
notice. Third-party files keep their original notices and attribution.
