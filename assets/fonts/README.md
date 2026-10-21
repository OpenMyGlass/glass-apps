# Fonts

[Inter](https://github.com/rsms/inter) by Rasmus Andersson and the Inter Project Authors, licensed under the SIL Open Font License 1.1 (see [`OFL.txt`](OFL.txt), copied unchanged from the release's `LICENSE.txt`). The font files are unmodified.

| | |
|---|---|
| Version | 4.1 |
| Release archive | https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip |
| Archive SHA-256 | `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e` |

| File | Path in the archive | SHA-256 |
|---|---|---|
| `Inter-Light.ttf` | `extras/ttf/Inter-Light.ttf` | `164414f0aacbe98a7e64addc43f7b3bfd2e32f7b90e101feeab227f14c371bda` |
| `Inter-Thin.ttf` | `extras/ttf/Inter-Thin.ttf` | `22453f995345e5618d539315a8501f4c74ca41d1898ce44e7d1b8206f0d097d4` |
| `Inter-Regular.ttf` | `extras/ttf/Inter-Regular.ttf` | `40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82` |
| `OFL.txt` | `LICENSE.txt` | `262481e844521b326f5ecd053e59b98c8b2da78c8ee1bdbb6e8174305e54935a` |

To check: `shasum -a 256 assets/fonts/*.ttf assets/fonts/OFL.txt`.

The UI uses Light for body and clock text, Thin for very large text, Regular where small text needs weight. Glass itself used Roboto; see the [plan](../../docs/plans/2026-10-21-r5-glass-apps.md#fonts) for why Inter is bundled instead.
