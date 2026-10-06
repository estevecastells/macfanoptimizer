# Hardware notes

Apple doesn't document the SMC. Everything here was found out by reading this machine's SMC directly (`fanctl keys`), by load experiments, and by watching how the system behaves. When you add facts, say how you found them.

## Mac17,9 (M5 Pro MacBook Pro), macOS 26

### Fans

| Key | Type | Meaning |
|---|---|---|
| `FNum` | `ui8` | Number of fans (2) |
| `F<n>Ac` | `flt` | Actual RPM |
| `F<n>Tg` | `flt` | Target RPM (writable) |
| `F<n>Mn` / `F<n>Mx` | `flt` | Min / max RPM: 2317 / 7826 |
| `F<n>md` | `ui8` | Mode: 0 = macOS controls, 1 = forced (writable). Lowercase `md` on Apple Silicon; Intel used `F<n>Md`. |
| `F<n>St` | `ui8` | Status (5 while running) |

**There is no `Ftst` key on the M5.** Tools written for M1–M4 write `Ftst=1` to unlock manual control. Here, forced mode works by writing `F<n>md=1` followed by `F<n>Tg`. We confirmed this by observing a third-party controller holding 6000 rpm with only those keys present. The daemon writes `Ftst` automatically on machines that do have it.

When fans are released (`md=0`), macOS keeps them off at idle and only ramps up late, when the die is already very hot.

**Validated with `fand` on 2026-10-06:** forced mode took effect within one tick (macOS's 2313 rpm → 3763 rpm commanded and reached). A 75 °C control temperature drove both fans to 5658 rpm within 3 s. Writing `md=0` handed control back to macOS immediately, and the fans began spinning down.

### Temperatures

There are 257 live `flt` temperature keys. The ones that rose under an all-core `yes` load test (25 s, fans fixed at 6000 rpm), most first:

| Prefix | Count | Under all-core load | Our label |
|---|---|---|---|
| `Tm` | 40 | +10–13 °C | CPU cluster (m) |
| `Ts` | 13 | +6–11 °C | SoC (s) |
| `Tp` | 23 | +5–6 °C (but the hottest single sensor during single-thread boost) | CPU P-cluster |
| `Tg` | 42 | GPU work | GPU |
| `TPD*`, `TRD*`, `TUD*` | | +7–9 °C | power delivery |
| `TVD0`, `TVDc`, `TVDM`, `TCMb` | | tracks the overall max | SMC-computed aggregates (not used) |
| `TVMX`, `TVms` | | constant 61 | thresholds/setpoints, not readings |

The controller uses the families `Tp Tm Ts Tg Te` (118 sensors on this machine) and takes the mean of the hottest 4.

Measured reference points:

- Idle: about 45–55 °C with fans off.
- Light all-core load at 6000 rpm: about 67 °C.
- Heavy sustained load (compiles plus background jobs) at 6000 rpm: about 100 °C hot spot. This is why the curves reach 100 % fan well before that.

### Cost

Each SMC read takes about 135 µs wall time and about 13 µs CPU on this machine, because Apple Silicon routes SMC requests through a firmware mailbox. A full scan of 118 sensors takes about 16 ms, and a hot-set tick about 3 ms.

## Validating a new model

1. Quit every other fan-control app.
2. `make build`, then collect `target/release/fanctl probe`, `fanctl keys F` and `fanctl sensors`. These are read-only.
3. Force a speed and watch it take effect:
   ```sh
   sudo target/release/fanctl fan set all 4000   # should report actual ≈ 4000 after 3 s
   sudo target/release/fanctl fan set all 99999  # clamps to max
   sudo target/release/fanctl fan release        # back to macOS; fans should spin down
   ```
   Note: `fanctl fan` refuses to write on models not in `SUPPORTED_MODELS`. For this test, temporarily add your model to the list in `crates/fand/src/lib.rs` and rebuild.
4. Install (`make install`), put the machine under load for 10 minutes (for example `yes > /dev/null` on every core, or a big compile), and check that `fanctl status` reaches max fans. Then stop the load and confirm the fans return to macOS (`reason Idle`) within a few minutes.
5. Check sleep/wake: sleep under load, wake, and confirm `fanctl status` shows the fans forced again within one tick.
6. Open a PR adding the model, with these outputs.
