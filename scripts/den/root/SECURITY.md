# Security policy

## Reporting a vulnerability

Report security issues privately through this repository's **Security →
Advisories → Report a vulnerability**. If you cannot use that, open an issue
that says only that you have something to report, with no details, and we
will reply with a private way to send it. Do not open a public issue for
anything that lets someone read a room, enter without a pass, act as another
member or device, or take a room or the voice service down.

Include the commit, the browser, exact steps, and the smallest test case
that shows it. Never include a real member pass or day key.

## Supported code

Only `main` receives fixes.

## Boundaries

[THREAT_MODEL.md](THREAT_MODEL.md) states what the server can and cannot see.
Anything it says the server cannot do, and the server can, is a vulnerability.
The den has not had an independent security audit.
