# Security policy

Portolano holds the keys to other people's servers, so security reports are taken seriously.
Thank you for looking.

## Supported versions

Portolano is in alpha. Only the **latest release** receives security fixes; there are no
backports to older versions. Please update before reporting, or say which version you used.

## How to report a vulnerability

Please report it **privately**, not in a public issue, a pull request or a discussion:

1. Open the repository on GitHub: <https://github.com/reallovedone/portolano>.
2. Go to the **Security** tab and choose **Report a vulnerability** (GitHub's private
   vulnerability reporting).
3. Fill in the form. Only you and the maintainer can see the report.

If you cannot see that button, open a normal issue that says only "I would like to report a
security problem privately" and nothing else, and the maintainer will arrange a private channel.
Do not put any details in that issue.

## What to include

- The version of Portolano (the file name of the installer shows it) and your operating system.
- What the problem is and what an attacker gains (for example: "a network attacker can read a
  stored password", "a crafted import file runs a program").
- The steps to reproduce it, or a small proof of concept. A crafted file or a short script is
  ideal. Please use test credentials and hosts you own.
- Whether you have told anyone else, and whether you plan to publish.

Please do not include real secrets, and do not test against servers you do not own.

## What to expect

- An acknowledgement within a few days. This is a personal open-source project maintained in
  spare time, so please be patient.
- A first assessment (is it a vulnerability, how severe) and, when it is one, a fix in the next
  release. You will be told when the fix is out, and credited in the release notes if you wish.
- Coordinated disclosure: please keep the details private until a fixed release is available.
  If a report goes unanswered for 90 days, you are free to publish it.

## Scope

In scope: the Portolano application and its own code (the vault, the connection document and
its import and export, SSH, RDP and VNC handling, the web view integration).

Out of scope: vulnerabilities in the operating system or in the servers you connect to, and
problems that need malware already running as your user (it can read what the application
reads). Known limits are listed in the README (section "Security") and in `deny.toml`
(the advisory of a dependency that has no fixed release yet).
