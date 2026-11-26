# RFCs

Design proposals for changes to the component API, file formats or the QEMU sync
protocol (see [`CONTRIBUTING.md`](../CONTRIBUTING.md)).

1. Copy [`0000-template.md`](0000-template.md) to `NNNN-short-title.md`, where
   `NNNN` is the next free four-digit number (look at existing files).
2. Fill in the header (title, author, date, status `Draft`, tracking issue) and
   every section.
3. Open a pull request. Reviewers move the status to `Accepted` or `Rejected`
   when the decision is made.

Smaller changes do not need an RFC; use an ordinary pull request.

| Number | Title | Status | Tracking |
|---|---|---|---|
| [0001](0001-component-api.md) | Component API for engine E2 | Accepted | [#46](https://github.com/sjnorval/Ohmforge/issues/46) |
| 0002 | Bus ports (reserved by [`buses.md`](../docs/design/buses.md), not written yet) | Reserved | — |
| [0003](0003-circuit-file.md) | Circuit file format (engine E4) | Implemented | [#48](https://github.com/sjnorval/Ohmforge/issues/48) |
| [0004](0004-instruments.md) | Instruments and the measurement service (engine E5) | Accepted | [#49](https://github.com/sjnorval/Ohmforge/issues/49) |
