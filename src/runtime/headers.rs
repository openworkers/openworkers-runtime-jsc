use rusty_jsc::JSContext;

/// Setup global Headers class
pub fn setup_headers(context: &mut JSContext) {
    let code = r#"
        globalThis.Headers = class Headers {
            constructor(init) {
                // A list, not a map: Set-Cookie holds one entry per cookie
                this._entries = [];

                if (init) {
                    if (init instanceof Headers || Array.isArray(init)) {
                        for (const [key, value] of init) {
                            this.append(key, value);
                        }
                    } else if (typeof init === 'object') {
                        for (const key of Object.keys(init)) {
                            this.append(key, init[key]);
                        }
                    }
                }
            }

            _normalizeKey(name) {
                return String(name).toLowerCase();
            }

            append(name, value) {
                const key = this._normalizeKey(name);
                const text = String(value);
                const existing = key === 'set-cookie'
                    ? undefined
                    : this._entries.find(([n]) => n === key);

                if (existing) {
                    existing[1] += ', ' + text;
                } else {
                    this._entries.push([key, text]);
                }
            }

            delete(name) {
                const key = this._normalizeKey(name);

                this._entries = this._entries.filter(([n]) => n !== key);
            }

            get(name) {
                const key = this._normalizeKey(name);
                const values = this._entries.filter(([n]) => n === key).map(([, v]) => v);

                return values.length === 0 ? null : values.join(', ');
            }

            has(name) {
                const key = this._normalizeKey(name);

                return this._entries.some(([n]) => n === key);
            }

            // Replaces in place so the header keeps its emission slot
            set(name, value) {
                const key = this._normalizeKey(name);
                const index = this._entries.findIndex(([n]) => n === key);

                if (index === -1) {
                    this._entries.push([key, String(value)]);
                    return;
                }

                this._entries[index] = [key, String(value)];
                this._entries = this._entries.filter(([n], i) => n !== key || i === index);
            }

            *entries() {
                for (const [key, value] of this._entries) {
                    yield [key, value];
                }
            }

            *keys() {
                for (const [key] of this._entries) {
                    yield key;
                }
            }

            *values() {
                for (const [, value] of this._entries) {
                    yield value;
                }
            }

            forEach(callback, thisArg) {
                for (const [key, value] of this._entries) {
                    callback.call(thisArg, value, key, this);
                }
            }

            [Symbol.iterator]() {
                return this.entries();
            }

            getSetCookie() {
                return this._entries
                    .filter(([key]) => key === 'set-cookie')
                    .map(([, value]) => value);
            }
        };

        // Native code enumerates own properties, which on a Headers yields its backing list
        globalThis.__normalizeHeaders = function(init) {
            const plain = {};

            if (!init) {
                return plain;
            }

            const headers = init instanceof Headers ? init : new Headers(init);

            for (const [name, value] of headers) {
                plain[name] = value;
            }

            return plain;
        };
    "#;

    context
        .evaluate_script(code, 1)
        .expect("Failed to setup Headers");
}
