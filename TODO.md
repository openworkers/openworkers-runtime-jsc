# TODO

## Web APIs

- [x] **Crypto API**
  - [x] `crypto.getRandomValues()`
  - [x] `crypto.randomUUID()`
  - [x] `crypto.subtle.digest()` (SHA-1, SHA-256, SHA-384, SHA-512)
  - [x] `crypto.subtle.sign()` / `verify()` (HMAC, ECDSA P-256, RSA PKCS#1 v1.5)
  - [x] `crypto.subtle.importKey()` (raw, pkcs8, spki)
  - [x] `crypto.subtle.generateKey()` (ECDSA P-256)

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
