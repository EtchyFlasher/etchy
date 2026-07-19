# Security Policy

Etchy writes raw bytes to physical disks and runs a small privileged helper
process — security reports are taken seriously and handled with priority.

## Reporting a vulnerability

**Please do not open a public issue for security vulnerabilities.**

Instead, use GitHub's private vulnerability reporting:

1. Go to the [Security tab](https://github.com/EtchyFlasher/etchy/security)
2. Click **"Report a vulnerability"**
3. Describe the issue, affected component (`etchy-core`, `etchy-helper`,
   `src-tauri`, or `ui`), and steps to reproduce

You'll get a response as soon as possible, and a fix will be prioritized —
especially for anything affecting device targeting/enumeration, the
elevation path, or the verification pipeline.

## Scope

Of particular interest:

- Bypasses of the device-safety filters (getting a system/internal disk listed or flashed)
- Privilege-escalation issues in `etchy-helper` or the elevation flow (pkexec / UAC / osascript)
- Verification bypasses (a flash reporting ✓ without a true byte-for-byte match)
- Injection via crafted ISO filenames/paths or IPC messages

## Supported versions

| Version | Supported |
|---|---|
| Latest release | ✅ |
| Older releases | ❌ — please update |
