# Bone-conduction audio: the stock gold reference (2026-11-03)

Captured on the owner's Glass, stock XE24 (XRH35) with the rooted boot image, **stock kernel
3.4.94**, while the stock audio HAL played UI sounds (`input keyevent 23`/`4` after
`input keyevent 26` to wake the screen). Then the same capture while *our* launcher played its
`--audio-stock-exact` tone on the same kernel. Raw files: `googleglass/out/gold-stock-audio-2026-11-03/`.

## Identical in both (so NOT the cause)

- PCM: `pcmC0D0p`, MMAP_INTERLEAVED, S16_LE, 48 kHz, 2 ch, period 14784, buffer 29568,
  start_threshold 2112, avail_min 14784.
- ABE: opp 25, `mm_dl1` 1/1, `pdmdl1` 1/1, `pdmul1` 1/1, everything else 0.
- DAPM: EP on, HSDAC Left on, HSDAC Right off, Earphone Driver on, PDM_DL1 on, Headset L/R off.
- Mixer: DL1 Mixer Multimedia 1, DL1 Media Playback Volume 120, DL1 PDM Switch 1,
  Earphone Playback Switch 1, Headset Playback Volume 12/12, Headset Power Mode 1,
  Sidetone Mixer Playback 1, DL1 Mono Mixer 0.
- TWL6040: NCPCTL 0x81, LDOCTL 0x45, HSLCTL/HSRCTL 0x01, HSGAIN 0x33, MICLCTL/MICRCTL 0x18, …

## The delta (TWL6040 regmap, read live during playback)

| register | stock (audible) | ours before the fix (silent) |
|---|---|---|
| 0x07 HPPLLCTL | `0x60` (MCLK 38.4 MHz selected, **HPLL disabled**) | `0x69` (HPLLENA + HPLLSQRENA) |
| 0x08 LPPLLCTL | `0x01` (**LPLL enabled**) | `0x10` (HPLLSEL, LPLL off) |
| 0x13 EARCTL | `0x01` (EP gain 15) | `0x07` (EP gain 12) |
| mixer `PLL Selection` | 0 = Low-Power | 1 = High-Performance |
| mixer `TWL6040 Power Mode` | 0 = Low-Power | 1 = High-Performance |

## Listening test (owner, same device, same kernel)

`glass-launcher --audio-analog` with countable patterns: V1 our HP preset = one long tone,
V2 stock power modes (`PLL Selection` 0, `TWL6040 Power Mode` 0) = two beeps, V3 HP + Headset
L/R on = three beeps. The owner heard **only the two beeps**. After switching
`prime_controls()` to the stock modes, the owner heard `--audio-selftest` (normal
AlsaFeedback path, writei, period 1056) on Android and, from a cold Linux boot of the Model B
image, the startup chime twice plus the UI clicks.

Conclusion: with the High-Performance PLL the codec is clocked from the 38.4 MHz MCLK, which is
not running on Glass in our stack, so every register/widget/port looked right while the DAC
had no clock. The Low-Power PLL runs from the 32 kHz clock and always works.

## Gotchas learned

- Reading `/sys/kernel/debug/regmap/1-004b/registers` while the codec is idle hangs the reader
  in D state on the stock kernel. Read it only during playback.
- The ASoC `codec_reg` cache does not reflect the PLL registers (it showed 0x60 in every
  variant); only the regmap debugfs read is real.
- On KitKat `adb shell` does not propagate exit codes; poll files instead.
