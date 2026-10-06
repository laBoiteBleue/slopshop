# Security policy

SlopShop opens files that may come from anyone (PSD, TIFF, EXR, archives…), so a malformed
file that crashes a decoder, makes it allocate without bound or read out of bounds is a
security problem.

## Reporting a vulnerability

Please do not open a public issue. Report it privately through GitHub:
[Report a vulnerability](https://github.com/laBoiteBleue/slopshop/security/advisories/new)
(the **Security** tab of the repository).

Include what you can: the affected version or commit, the file or steps that trigger the
problem, and what you observed. You will get an answer as soon as possible; SlopShop is
maintained by one person, so please allow a few days.

## Supported versions

SlopShop has no stable release yet: only the latest `main` is supported.
