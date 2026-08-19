use rusty_jsc::JSContext;

/// Setup FormData and the body decoder Request/Response.formData() calls
pub fn setup_form_data(context: &mut JSContext) {
    let code = r#"
        globalThis.FormData = class FormData {
            constructor() {
                this._entries = [];
            }

            append(name, value) {
                this._entries.push([String(name), String(value)]);
            }

            delete(name) {
                const key = String(name);

                this._entries = this._entries.filter(([n]) => n !== key);
            }

            get(name) {
                const key = String(name);
                const hit = this._entries.find(([n]) => n === key);

                return hit === undefined ? null : hit[1];
            }

            getAll(name) {
                const key = String(name);

                return this._entries.filter(([n]) => n === key).map(([, v]) => v);
            }

            has(name) {
                const key = String(name);

                return this._entries.some(([n]) => n === key);
            }

            set(name, value) {
                const key = String(name);
                const index = this._entries.findIndex(([n]) => n === key);

                if (index === -1) {
                    this._entries.push([key, String(value)]);
                    return;
                }

                this._entries[index] = [key, String(value)];
                this._entries = this._entries.filter(([n], i) => n !== key || i === index);
            }

            *entries() {
                for (const [name, value] of this._entries) {
                    yield [name, value];
                }
            }

            *keys() {
                for (const [name] of this._entries) {
                    yield name;
                }
            }

            *values() {
                for (const [, value] of this._entries) {
                    yield value;
                }
            }

            forEach(callback, thisArg) {
                for (const [name, value] of this._entries) {
                    callback.call(thisArg, value, name, this);
                }
            }

            [Symbol.iterator]() {
                return this.entries();
            }
        };

        globalThis.__parseFormData = function(text, contentType) {
            const type = String(contentType || '').split(';')[0].trim().toLowerCase();

            if (type !== 'application/x-www-form-urlencoded') {
                throw new TypeError(`Cannot decode a ${type || 'typeless'} body as FormData`);
            }

            const form = new FormData();

            // URLSearchParams already applies the urlencoded rules, '+' included
            for (const [name, value] of new URLSearchParams(text)) {
                form.append(name, value);
            }

            return form;
        };
    "#;

    context
        .evaluate_script(code, 1)
        .expect("Failed to setup FormData");
}
