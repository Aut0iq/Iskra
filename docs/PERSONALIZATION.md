# Personalization boundary and roadmap

Shipped foundation: declarative avatar packs, portrait initialization,
validation, attribution and custom unsigned plugin builds. Packs contain
images/JSON, never scripts. No network, account, command or signing access
is granted by choosing artwork.

`schemas/theme.schema.json` defines a future palette pack. Values are
restricted to colors, not CSS, URLs or executable expressions.
`schemas/personalization.schema.json` reserves avatar/theme IDs and a
reduced-motion preference. These schemas are versioned interchange contracts,
not a claim that the plugin currently applies themes or switches avatars at
runtime. Existing reduced-motion behavior still follows the host.

Next implementation steps: runtime pack discovery through a reviewed host
file-access boundary, accessible theme tokens, settings and previews,
reduced-motion/static fallbacks, then an explicit import flow. Each step
needs its own permissions review and tests before being called shipped.
No avatars, packs or remote URLs are fetched automatically.
