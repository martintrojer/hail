# Speakeasy design

Speakeasy is a Screener bypass password/passphrase, not a sender allow-list and
not route management.

## Product semantics

- Hail maintains one current Speakeasy password/passphrase for the mailbox.
- The password/passphrase rotates monthly.
- When an incoming email includes the current password/passphrase, that message
  bypasses the Screener and is not held for sender approval.
- The bypass applies to the matching message only. It does not approve the
  sender for future messages, add an allowed-sender rule, or choose an Imbox /
  Feed / Paper Trail route.
- Existing Screener sender decisions remain the mechanism for approving,
  blocking, and routing future mail from a sender.

## Open implementation choices

These details should be decided by the implementation tasks before shipping:

- **Delivery behavior:** where a bypassed message appears after skipping sender
  approval, without turning Speakeasy itself into route management.
- **Matching surface:** whether to scan subject, text body, HTML body, and/or
  selected headers.
- **Generation and rotation:** how passphrases are generated, when monthly
  rotation happens, and whether the previous passphrase has any grace period.
- **Visibility:** where the current passphrase appears in the UI and how clearly
  the UI communicates that sharing it grants a one-message Screener bypass.
- **Audit/security:** whether to record bypass events, how to avoid logging the
  passphrase, and how to handle forwarded/replied messages that may contain an
  old passphrase.
