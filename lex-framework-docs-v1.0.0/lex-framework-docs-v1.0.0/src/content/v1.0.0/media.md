# Media storage and malware scanning

Lex exposes provider-neutral media interfaces so models/views do not depend on a specific cloud SDK.

## Filesystem development storage

```toml
[media]
storage = "filesystem"
local_root = "./media"
```

## In-memory test storage

```toml
[media]
storage = "memory"
```

Memory storage is intended for tests and is rejected by production validation.

## S3 / Backblaze B2 / MinIO

```toml
[media]
storage = "s3"

[media.s3]
endpoint = "https://s3.us-west-004.backblazeb2.com"
region = "us-west-004"
bucket = "my-private-media"
access_key_env = "LEX_MEDIA_S3_ACCESS_KEY_ID"
secret_key_env = "LEX_MEDIA_S3_SECRET_ACCESS_KEY"
```

Credentials are supplied outside `Config.toml`.

## ClamAV

```toml
[media]
scanner = "clamav"

[media.clamav]
host = "127.0.0.1"
port = 3310
timeout_secs = 15
fail_closed = true
```

Recommended upload lifecycle:

```text
receive upload
  → validate size/type
  → quarantine/temp state
  → scan with ClamAV
  → store/activate only when clean
```
