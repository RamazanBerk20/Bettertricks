# Security policy

Bettertricks downloads installers and runs Windows programs, so recipe changes and update
verification are security-sensitive.

## Reporting

Do not open a public issue for a vulnerability. Send a private report to the security contact
listed in the repository hosting configuration. Include the affected version, reproduction,
impact, and whether a malicious catalog, recipe, archive, prefix, or local file is required.

## Security boundaries

- Bundled recipes and catalogs activated through a configured Ed25519 trust root are trusted.
- Metadata-only catalog recipes execute only as a validated catalog-owned identifier through a
  checksum-pinned Winetricks host whose version exactly matches the active catalog baseline.
  Missing, modified, or mismatched hosts fail closed before Wine starts.
- Downloads are checksum-verified before use.
- Catalog archives reject traversal, links, and special files before activation.
- The managed compatibility host is downloaded only over HTTPS, capped at 4 MiB, verified against
  a release-pinned SHA-256 before atomic publication, re-verified before every use, and never
  invoked through a shell.
- Custom `.verb` files are arbitrary shell code. They require explicit trust and an available
  Winetricks compatibility host; Bettertricks creates a restore point first.
- Wine prefixes are not security sandboxes. Run untrusted Windows software in an OS-level
  sandbox or virtual machine.

Never publish a catalog signing seed, add it to repository secrets visible to pull requests,
or pass it directly on a command line. Rotate the public trust root in an application release
if a signing seed may have been exposed.
