# Official examples and sharing a project

This page previews the planned gallery and the file-based project pack. It does
not work yet. The design is in
[`docs/design/examples-and-sharing.md`](https://github.com/sjnorval/Ohmforge/blob/main/docs/design/examples-and-sharing.md).
Circuit files (E4) are on `main`; the parts library on the circuit (E6) and the
gallery itself still have to land. For what you can do **today**, use the bundled
demos in the desktop app, or `ohmforge run` on a circuit file; see
[Your first schematic](first-schematic.md#what-works-today).

## Open an official example

*Needs E4 (done), E6, and the P3 gallery.*

On the load screen, or under File → Examples, pick a project: title, chip, a
short description, and a beginner or intermediate mark. Ohmforge copies it into
a folder you own and opens it, so the original stays unchanged. Each example
has a README that says what the circuit does, how to rebuild the firmware, and
what to try next.

Release 0.2 aims for at least five official examples, including the blink,
button, dimmer and fade circuits you can already run as built-in demos, plus
one slightly richer board.

## Export a pack and send the file

*Needs the P3 pack format.*

File → Export pack (or `ohmforge pack <project>`) writes a zip or `.ohmforge`
archive of the project folder: the circuit, firmware or a rebuild recipe, notes,
and any prices you typed yourself. File → Open pack (or `ohmforge unpack`)
opens it on another machine, offline, with no account. Supplier API keys and
other secrets are not included.

A pack can also be opened from a path or an HTTPS URL of the archive. The
desktop app accepts that in P3; the web app (P4) will too, after asking before
it downloads anything.

## What this is not

Sharing does not wait on a cloud account. Accounts and cross-device sync stay a
parked future idea. There is no Ohmforge-hosted gallery to upload to, and
packs are not a live multi-user session.
