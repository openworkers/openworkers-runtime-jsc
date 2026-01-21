# TODO

## Web APIs

- [x] **Crypto API**
  - [x] `crypto.getRandomValues()`
  - [x] `crypto.randomUUID()`
  - [x] `crypto.subtle.digest()` (SHA-1, SHA-256, SHA-384, SHA-512)
  - [x] `crypto.subtle.sign()` / `verify()` (HMAC)
  - [x] `crypto.subtle.importKey()` (raw format for HMAC)
  - [ ] `crypto.subtle.sign()` / `verify()` (ECDSA, RSA) — optional

- [ ] **Blob / File**
  - [ ] `Blob` constructor and methods
  - [ ] `File` constructor

- [ ] **FormData**
  - [ ] `FormData` constructor and methods

- [ ] **AbortController**
  - [ ] `AbortController`
  - [ ] `AbortSignal`
  - [ ] fetch with signal support

- [ ] **Other APIs**
  - [ ] `structuredClone()`
  - [ ] `performance.now()`

## Bindings

- [ ] **Assets** — `ASSETS.fetch(path, options)`
- [ ] **Storage** — `BUCKET.get()`, `put()`, `head()`, `list()`, `delete()`
- [ ] **KV** — `KV.get()`, `put()`, `delete()`, `list()`
- [ ] **Database** — `DB.query()`
- [ ] **Worker** — `WORKER.fetch(options)`
