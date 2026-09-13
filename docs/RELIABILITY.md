# Availability and fault recovery

The requirement is to keep sampling and serving whenever the hardware and LAN permit, and make missing data visible. Other systems detect unreachability. No injury/damage hazard was identified by the user. This firmware has not run on a board; the following is implemented behavior and a test plan, not demonstrated hardware reliability.

## Supervision

`esp-hal 1.2.1` disables watchdogs in `init()`. Board setup immediately arms the **RTC watchdog**, before logging, peripheral setup or an await. The HAL configures stage 0 to reset the full digital system and RTC, using the independent RTC slow clock. It is nominally **30 seconds**; measure actual timing across the expected voltage/temperature range. Deep/light sleep is not used. There is no interrupt that blindly feeds the watchdog.

The generic core of `task-watchdog 0.1.2` tracks five distinct task IDs. Our adapters provide the current Embassy monotonic clock and official HAL RTC driver. An Embassy `Ticker` checks once per second and feeds hardware only when every registered task is within its deadline. A starvation result is latched: the supervisor never feeds again, even if a delayed task subsequently reports progress. If the executor, interrupts, or supervisor itself stalls, the hardware watchdog remains armed.

| Task | Progress evidence | Deadline since last progress | Nominal reset bound after last progress if only this task stalls |
| --- | --- | --- | --- |
| Sensor | Completed sample attempt, publication, and bounded error recovery | 20 s | Approximately 51 s |
| Network | Actual timed PHY/MDIO poll completed inside the network runner | 10 s | Approximately 41 s |
| HTTP worker 0 | Completed/failed/timed-out connection or bounded idle accept cycle | 210 s | Approximately 241 s |
| HTTP worker 1 | Same, independently registered | 210 s | Approximately 241 s |
| Periodic task | Completed 60-second tick | 90 s | Approximately 121 s |

Bounds include up to one second to detect starvation and a conservative additional 30-second hardware period. Reboot, PHY startup and DHCP recovery add downtime after reset. A CPU/interrupt/executor stall prevents hardware feeding immediately and should reset within approximately 30 seconds of the last hardware feed. The bounds depend on RTC clock tolerance and must be measured. Startup must finish before the first 30-second hardware deadline.

Progress confirms that these operations ran, not that every driver/stack invariant is healthy. A runner that still polls its PHY but has a logical TCP bug may satisfy supervision. The watchdog is one recovery layer, not a proof of correct networking.

## External faults and bounded work

- **Missing sensor / CRC error:** publish `last_ok=false`, increment a saturating error counter, attempt the driver's soft reset with a 100 ms bound, and retry on the next 5-second tick. A read has a 250 ms application timeout. The HAL supplies I²C state-machine recovery; future cancellation can briefly block in its cleanup. A hardware/driver lockup that blocks the executor remains covered by the RTC watchdog. Missing hardware alone does not cause repeated board resets.
- **Stale data:** `/api/reading` retains the previous sample and age but returns 503 and `fresh:false` after a failed attempt or after 15 seconds. Initial values are null. `/ready` requires a fresh reading plus IPv4. `/health` only means the HTTP application can answer. Monitors must enforce their own request timeout and detect unreachable devices.
- **Cable or DHCP outage:** keep sensor and application tasks running. Embassy-net owns address removal, reacquisition and lease renewal. The PHY timer continues to wake and exercise the runner without a link. A reset is not triggered just because no link or lease exists.
- **Slow client:** 10-second initial/ordinary request timeout, 2-second write timeout, 15-second TCP inactivity timeout, and 190-second absolute connection limit. Two clients can consume both workers temporarily. This small server is not designed to withstand an adversarial connection flood.
- **OTA:** two workers allow one upload and normal reads concurrently, with brief pauses during synchronous flash operations. Accepted upload bodies and the update service are bounded to 180 seconds. Picoserve retains the short request deadline for draining rejected bodies; rejection need not be instantaneous. Concurrent uploads return 503. Cancellation/error releases the flash mutex. After metadata commit, the image is protected from a second upload and an independent Embassy task resets after two seconds, regardless of response delivery.
- **Panic / lost task / blocked HAL operation:** allow hardware reset rather than keeping a timer-only heartbeat alive. Boot logs include the HAL reset reason. Counters and samples are RAM-only and restart on boot.

## Remaining availability limits

Automatic rollback from a valid but unhealthy OTA application is **not implemented**. The standard bundled bootloader does not perform application-health rollback. Image identity, SHA256, authentication and inactive-slot writes prevent several classes of bad update, but cannot establish that new application code boots and operates correctly. A bad signed image can cause a reset loop requiring serial recovery. A rollback-enabled standard bootloader and a tested first-boot confirmation policy remain necessary before relying on unattended remote updates.

Board reset does not remove power from the SHT40 or explicitly reset the LAN8720A. GPIO16 controls the Ethernet oscillator, not PHY reset. A latched peripheral fault may require a physical power cycle. No sensor power-control hardware is assumed. The supply, power-on/brownout behavior, flash chip, exact board revision and direct I²C cable are unverified. Software cannot keep a single unpowered board available or guarantee an electrically marginal cable. Persistent history, redundant sensing and seamless updates are outside this implementation.

## Required hardware fault-injection tests

Run these on a spare/test unit with serial recovery available; never infer a pass from the host tests.

1. Log chip revision and reset reason. Verify startup completes within the watchdog budget with and without Ethernet and sensor attached.
2. Build temporary fault-injection variants that stop each monitored task immediately after a heartbeat while other tasks continue. For each, confirm reset within the corresponding measured bound, then normal boot and service recovery. Restore normal firmware afterward.
3. Inject a CPU spin and an interrupt-disabled spin. Confirm RTC reset without an executor or interrupt callback. Repeat under a sustained network load and while flash writes are active.
4. Delay one task beyond its deadline, then allow it to report progress before the RTC expires. Confirm starvation remains latched and the board still resets.
5. Disconnect/reconnect the sensor and LAN repeatedly, independently and together. Confirm continued boot uptime during ordinary external faults, correct 503/error/age reporting, and automatic recovery once the fault is removed. Test SDA held low and SCL held low safely, including removal of the fault.
6. Test DHCP unavailable at boot, lease expiry, server restart and changed addresses. Confirm the sensor and 60-second task continue and networking returns without a reboot loop.
7. Run parallel reading requests during a maximum-size OTA. Attempt a second signed upload during transfer and again after commit. Confirm only one update proceeds, status remains responsive when a worker is available, and the committed image is not overwritten.
8. Stall clients at the request line, headers and body; omit the remaining body of a rejected upload; stop reading responses. Test both HTTP workers occupied, then verify they recover after deadlines. Disconnect the OTA client immediately after commit; confirm reboot still occurs.
9. Cut power during inactive-image writes and both metadata-sector operations. Verify recovery and recorded boot choice. Confirm serial rescue after a deliberately valid but nonfunctional test image; no automatic health rollback is expected in this version.
10. Soak the complete assembly and actual cable for at least 72 hours with expected traffic, repeated link faults and representative supply/environmental conditions. Collect remote request success, sample/error/age history, reset reasons and recovery times. Define acceptance from measured behavior, not the absence of visible serial errors.

Host tests exercise the actual community watchdog core with a simulated clock/hardware: initial grace, each task independently missing its deadline, and healthy progress over ten simulated minutes. HTTP tests exercise actual picoserve routes for readiness, missing address, stale data and busy-update status. They do **not** test the RTC peripheral, interrupt/executor stalls, two on-device workers, flash mutex/cancellation, or physical recovery.
