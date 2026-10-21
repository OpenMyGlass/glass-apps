# Fonts

[Inter](https://github.com/rsms/inter) by Rasmus Andersson and the Inter Project Authors, licensed under the SIL Open Font License 1.1 (see [`OFL.txt`](OFL.txt), copied unchanged from the release's `LICENSE.txt`). The font file is unmodified.

| | |
|---|---|
| Version | 4.1 |
| Release archive | https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip |
| Archive SHA-256 | `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e` |

| File | Path in the archive | SHA-256 |
|---|---|---|
| `Inter-Light.ttf` | `extras/ttf/Inter-Light.ttf` | `164414f0aacbe98a7e64addc43f7b3bfd2e32f7b90e101feeab227f14c371bda` |
| `OFL.txt` | `LICENSE.txt` | `262481e844521b326f5ecd053e59b98c8b2da78c8ee1bdbb6e8174305e54935a` |

To check: `shasum -a 256 assets/fonts/Inter-Light.ttf assets/fonts/OFL.txt`.

The UI uses only Inter Light (all card text). Other weights from the same release (for example `extras/ttf/Inter-Thin.ttf`) can be added here, with their checksum, when the UI needs them. Glass itself used Roboto; see the [plan](../../docs/plans/2026-10-21-r5-glass-apps.md#fonts) for why Inter is bundled instead.
