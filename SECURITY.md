# Security policy

## Reporting a vulnerability

Please **do not open a public issue** for a security problem. Report it privately instead:

1. Go to the [Security tab](https://github.com/RafaelGoulartB/Heronite/security) of this repository.
2. Click **Report a vulnerability** and fill in the form.

Only the maintainers can see the report. Include what an attacker could do, the steps to reproduce it, and the Heronite version (**Settings › About**).

You will get an answer within 7 days. Once the problem is fixed and a release is out, the advisory is published with credit to you, unless you prefer to stay anonymous.

## Supported versions

Security fixes go into the latest release. Please update to it before reporting.

## What counts

Heronite handles data that must stay private and starts programs on your machine. Reports about these are especially welcome:

- **Credentials:** the Epic Games session (legendary's `user.json` in the data folder), IGDB keys, and other tokens Heronite stores or sends.
- **Running programs:** anything that makes Heronite run a command or executable the user did not choose, for example through launch options, game metadata, `heronite://` links (and the older `heronite-rs://`), or downloaded Wine/Proton builds.
- **Files:** writing or deleting files outside the folders Heronite is meant to touch.
- **Network:** sending data to a server the user did not expect.

Problems in Wine, Proton, legendary, Ludusavi, or a game itself should go to those projects.
