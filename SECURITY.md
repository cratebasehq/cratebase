# Security Policy

## Reporting a vulnerability

Please report security vulnerabilities privately, not through a public
GitHub issue.

Use GitHub's private vulnerability reporting flow for this repository:

**https://github.com/cratebasehq/cratebase/security/advisories/new**

This opens a draft security advisory that only the maintainers can see
until it's ready to be disclosed. Include:

- A description of the vulnerability and its impact.
- Steps to reproduce it (a minimal repro is ideal — e.g. a `curl` sequence
  or a small script against a fresh `cratebase serve` instance).
- The version/commit you tested against.
- Any suggested fix or mitigation, if you have one.

We don't currently have a dedicated security contact email — the GitHub
advisory flow above is the primary and preferred channel. Please don't
open a regular public issue or pull request for a suspected vulnerability
before it's been triaged privately.

## Supported versions

Cratebase is at an early, pre-1.0 stage (currently `0.1.0`) with no
long-term-support policy yet. Until the project reaches a more stable
release cadence, only the latest code on the `main` branch and the most
recent tagged release are supported — please reproduce against current
`main` before reporting, and expect security fixes to land there rather
than being backported to an older point-in-time release.

## Scope

This applies to the Cratebase server (`crates/*`), the admin dashboard
(`web/admin`), the email templates (`web/email`), and the published SDK
package (`sdk/js/extras`). Vulnerabilities in third-party dependencies
should generally be reported upstream first, but let us know here too if
they're reachable through Cratebase's own code paths — see
`.github/workflows/ci.yml`'s `dependency-scan` job for how dependency
advisories are currently tracked (`cargo audit` / `bun audit`).
