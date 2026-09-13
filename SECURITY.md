# Security policy

## Supported versions

Security fixes are applied to the latest version on the default branch.

## Reporting a vulnerability

Contact the repository owner directly. Do not open an issue for a suspected
security vulnerability. Include a clear description, reproduction steps,
impact, and any suggested mitigation.

GitHub offers repository security advisories and private vulnerability
reporting for public repositories on GitHub.com only, so that route is not
available while this repository stays private.

Use fabricated catalogue pages, source configuration and reproduction data only. Do
not submit client or private catalogue data, credentials, access tokens, cookies,
secrets or `.env` content.

## What this tool does and does not do

DiogenesLamp fetches public Yupoo catalogue pages, writes JSON and thumbnails
to a local directory, and serves a local viewer. It does not log in, hold
OAuth tokens, or write back to Yupoo.

Treat scraped titles, HTML and JSON as untrusted data in the viewer. Do not
run the importer with elevated privileges, and do not point it at a less
trusted user's working directory.

The scraper talks to the network. `lamp import` reads a local link dump and
writes `sources.toml` without fetching anything.

Keep `sources.toml` free of credentials. The expected input is a dump of public
http or https links: Yupoo catalogue URLs, which are the only ones scraped, plus
non-Yupoo shop links (taobao, tmall, weidian) and link aggregators, which are
kept as a link-out directory and never fetched.
