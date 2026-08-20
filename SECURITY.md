# Security policy

## Supported versions

Security fixes are applied to the latest version on the default branch.

## Reporting a vulnerability

Please use this repository's private vulnerability reporting feature. Do not
open a public issue for a suspected security vulnerability. Include a clear
description, reproduction steps, impact, and any suggested mitigation.

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

The importer talks to the network. Keep `sources.toml` free of credentials;
Yupoo catalogue URLs are the only expected input.
