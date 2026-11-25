# Wireless simulation

This page will become the user-facing fidelity contract for the virtual radio.
It is a preview; there is no Wi-Fi or BLE model on `main` today. The design is
in [`docs/design/wireless.md`](https://github.com/sjnorval/Ohmforge/blob/main/docs/design/wireless.md).
The radio itself is a P4 item; BLE is a P6 stretch.

## Default: a virtual network

Simulated boards share a virtual medium. Official examples will run offline:
two boards, or one board and a small service that does not route to a real
network. Nothing in the default project configuration talks to the public
Internet.

## Optional host bridge

A project option can enable a host bridge so firmware may talk to services on
your machine. That option is explicit, visible in the project, and off by
default.

## The fidelity table

The finished page will list scan and join, DHCP, DNS, IPv4, TCP, UDP, TLS,
HTTP, MQTT and similar rows as Yes, Partial (with the limit), or No, with the
test that backs each Yes. When a row is No, the docs will say so rather than
implying a complete stack.
