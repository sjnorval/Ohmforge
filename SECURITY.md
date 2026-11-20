# Security policy

## Supported versions

Ohmforge is pre-alpha (before 1.0). Only the latest `main` branch is supported; there are no maintained release branches yet, and fixes land on `main`.

| Version | Supported |
|---|---|
| `main` | Yes |
| Anything older | No |

## Reporting a vulnerability

Please **do not** report security problems in public issues, pull requests or Discussions.

Report them privately, either way:

- **GitHub private vulnerability reporting** (preferred): open the [Security tab](https://github.com/sjnorval/Ohmforge/security) and choose "Report a vulnerability", or go straight to [a new advisory](https://github.com/sjnorval/Ohmforge/security/advisories/new).
- **Email**: [sjnorval@gmail.com](mailto:sjnorval@gmail.com), with "Ohmforge security" in the subject.

Include what you found, how to reproduce it (for example a crafted ELF, flash image, netlist or `part.toml`), the impact you expect, and the commit you tested.

Things in scope include memory-safety or panic issues triggered by untrusted input files (firmware images, netlists, parts, project files), sandbox escapes from WASM part models once they exist, and problems in CI or release workflows. Bugs in third-party crates are best reported upstream too; we track advisories with `cargo deny`.

## What to expect

Ohmforge is maintained by one person in their spare time, so these are targets, not guarantees:

- An acknowledgement within 7 days.
- A first assessment (confirmed or not, and how serious) within 14 days.
- A fix or mitigation on `main` as soon as practical, then a GitHub security advisory crediting you unless you prefer not to be named.

Please give us a reasonable chance to fix the problem before disclosing it publicly. We will coordinate the disclosure date with you.
