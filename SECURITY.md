# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.2.x   | ✅        |
| < 0.2   | ❌        |

## Reporting a vulnerability

Please report security issues privately through GitHub's
[private vulnerability reporting](https://github.com/redpotion39/wflow/security/advisories/new)
instead of opening a public issue.

## How Wflow handles secrets

* API keys and session tokens are **never** written to `config.json`. They are
  stored in a separate `secrets.json` file next to it, created with `0600`
  permissions on Unix and written atomically.
* `config.json` uses `skip_serializing` for every secret field, so a leaked
  config file never contains credentials.
* Secrets can be overridden per-process with environment variables
  (`WFLOW_AUTH_TOKEN`, `WFLOW_GROQ_API_KEY`, `WFLOW_ELEVENLABS_API_KEY`,
  `WFLOW_BASETEN_API_KEY`) which take precedence over the stored values.
* The gRPC gateway key is **not** embedded in the binary. It must be supplied
  through Settings or the environment.
* Diagnostic logs are written to the platform log directory and never contain
  API keys or raw transcripts.

## Privacy

* By default Wflow does **not** store the contents of the focused text box in
  the local history (`store_textbox_contents = false`).
* The surrounding text context is only transmitted to the speech-to-text
  provider when `share_context` is enabled. Disable it in
  **Settings → Privacy & Local Data** if you dictate sensitive text.
* All history stays in a local SQLite database that you can erase at any time
  with **Settings → Privacy & Local Data → Clear history** (or by deleting
  `flow.sqlite`).

## Rotating a compromised key

1. Revoke the key in the provider's console (Groq / ElevenLabs / Baseten).
2. Delete `secrets.json` in the Wflow config directory, or clear the field in
   Settings.
3. Enter the new key in Settings, or export the environment variable.
