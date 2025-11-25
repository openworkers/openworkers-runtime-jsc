use rusty_jsc::JSContext;

/// Setup global Headers class
pub fn setup_headers(context: &mut JSContext) {
    let code = r#"
        globalThis.Headers = class Headers {
            constructor(init) {
                this._map = new Map();

                if (init) {
                    if (init instanceof Headers) {
                        // Copy from another Headers object
                        for (const [key, value] of init) {
                            this._map.set(key, value);
                        }
                    } else if (Array.isArray(init)) {
                        // Array of [key, value] pairs
                        for (const [key, value] of init) {
                            this.append(key, value);
                        }
                    } else if (typeof init === 'object') {
                        // Plain object
                        for (const key of Object.keys(init)) {
                            this.append(key, init[key]);
                        }
                    }
                }
            }

            // Normalize header name (lowercase)
            _normalizeKey(name) {
                return String(name).toLowerCase();
            }

            append(name, value) {
                const key = this._normalizeKey(name);
                const strValue = String(value);
                if (this._map.has(key)) {
                    this._map.set(key, this._map.get(key) + ', ' + strValue);
                } else {
                    this._map.set(key, strValue);
                }
            }

            delete(name) {
                this._map.delete(this._normalizeKey(name));
            }

            get(name) {
                const value = this._map.get(this._normalizeKey(name));
                return value !== undefined ? value : null;
            }

            has(name) {
                return this._map.has(this._normalizeKey(name));
            }

            set(name, value) {
                this._map.set(this._normalizeKey(name), String(value));
            }

            // Iteration methods
            *entries() {
                yield* this._map.entries();
            }

            *keys() {
                yield* this._map.keys();
            }

            *values() {
                yield* this._map.values();
            }

            forEach(callback, thisArg) {
                for (const [key, value] of this._map) {
                    callback.call(thisArg, value, key, this);
                }
            }

            // Make Headers iterable
            [Symbol.iterator]() {
                return this.entries();
            }

            // getSetCookie returns all Set-Cookie headers as array
            getSetCookie() {
                const cookies = [];
                const value = this._map.get('set-cookie');
                if (value) {
                    cookies.push(value);
                }
                return cookies;
            }
        };
    "#;

    context
        .evaluate_script(code, 1)
        .expect("Failed to setup Headers");
}
