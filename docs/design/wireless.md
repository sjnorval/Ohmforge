# Wireless simulation and fidelity documentation

Status: proposed (design only, no code). The roadmap already has a P4 “Wi-Fi virtual radio (simulated network between boards and a host bridge)” and a P6 BLE stretch. This note is about what that radio is, and about the documentation that must ship with it.
Date: 2026-11-25.
Related: [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §8–§10; [`editor-integration.md`](editor-integration.md); [`examples-and-sharing.md`](examples-and-sharing.md); [`ROADMAP.md`](../ROADMAP.md) P4 and P6.

Wi-Fi and BLE are easy to over-promise. This feature is a **virtual radio** with a stated, tested set of behaviours, and a user-facing fidelity page that lists what works, what is approximate, and what never leaves the machine unless the user opts into a host bridge.

## 1. Current state

The ESP32-C3 model has no Wi-Fi or BLE radio. Nothing on `main` accepts `wifi_*` or BLE calls. The P4 item names a simulated network between boards and a host bridge, but there is not yet a behaviour list or a documentation plan.

## 2. Goals and non-goals

Goals:

- A virtual 802.11-like station and soft-AP that firmware can join, so two simulated boards (and, optionally, a host-side bridge) can exchange the traffic we claim to support.
- A written fidelity contract in the user guide, shipped in the same release as the radio: APIs and protocols that work, addressing and DNS, TLS, timing, and whether any packet can reach a real network.
- Off by default for any path to a real network. A host bridge is an explicit opt-in, with a clear prompt, and is never required for the official examples.
- The same standard for BLE when the P6 stretch is taken: a fidelity page first, not an implied complete controller.

Non-goals:

- Bit-accurate RF, coexistence, or certification-grade protocol behaviour.
- Silently reaching the public Internet from firmware that calls `WiFi.begin`.
- Blocking P4 on BLE.

## 3. User-facing behaviour

In the default mode, simulated boards share a virtual medium. Official examples show two stations, or one station and a small in-process service (HTTP or MQTT) on the host side of a **simulated** bridge that does not route to the real network. A settings or project option enables a host bridge that can bind to a local interface; the UI says, in one sentence, that firmware may then talk to services on the user’s machine or network, and the option is stored in the project, not hidden in an environment variable alone.

The fidelity page (mdBook, also linked from the radio’s inspector) is a table: join and scan, DHCP, DNS, IPv4, TCP, UDP, TLS, HTTP, MQTT, ICMP, SoftAP, power-save, 802.11 protocols, and “talks to a real LAN”. Each row is Yes, Partial (with the limit), or No, with the firmware API family used to test it. When a row is No, the firmware call fails in a way the real stack would understand where we can, or the official docs say it is unimplemented.

## 4. Technical approach

- Implement the radio as an SoC component with a host-side virtual medium in the engine, not as a pass-through of the user’s Wi-Fi card.
- Record traces (frames or socket-level events, *planned*) so a test can assert “this board sent a DHCP request” without a real network.
- Tests run offline in CI for every Yes on the fidelity table. Partial rows have a test for the stated limit.
- The fidelity page is generated from, or checked against, the same table the tests use, so the docs cannot drift. Licence: the page is CC-BY-SA-4.0.

## 5. Phasing

- **P4 / 0.3:** virtual medium between boards; a documented, opt-in host bridge; fidelity page for Wi-Fi; official example that does not need the host bridge. Depends on enough of the C3 (and then classic ESP32) radio and network offload being modelled to boot the official stacks we claim to support — the implementation issue must list those registers and shims.
- **P6:** BLE stretch, with its own fidelity page, or a written decision to defer BLE past 1.0.

## 6. Acceptance criteria

- The official Wi-Fi example runs offline in CI.
- Every Yes or Partial row on the fidelity page has a test; every No is explicit.
- The default project configuration performs no real-network I/O.
- Enabling the host bridge requires an explicit user action and is visible in the project.

## 7. Open questions

- How much of the ESP-IDF / Arduino Wi-Fi API needs a full MAC model versus a socket-level shim, and how that choice is described on the fidelity page so it is not mistaken for RF-accurate simulation.
- Whether the first host bridge is a userspace stack (safer, easier to test) or a tap/TUN device.
