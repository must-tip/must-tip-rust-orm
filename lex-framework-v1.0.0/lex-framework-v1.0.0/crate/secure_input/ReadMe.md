# secure_input

Input normalization, validation, sanitization, and context-specific output encoding primitives.

Security-sensitive rules are centralized in the canonical validator/sanitizer modules and use `std::sync::LazyLock` for compiled patterns. HTML sanitization uses Ammonia; JSON/XML/JavaScript/CSS/URL output paths have separate encoders.

Lexical SQL/XSS detection is defense-in-depth and does not replace parameterized SQL or sink-specific encoding.
