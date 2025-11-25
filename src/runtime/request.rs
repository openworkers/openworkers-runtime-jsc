/// Request class implementation (WHATWG Fetch API)
pub const REQUEST_JS: &str = r#"
    globalThis.Request = class Request {
        constructor(input, init) {
            init = init || {};

            // Handle input - can be a URL string or another Request
            if (input instanceof Request) {
                // Clone from another Request
                this.url = input.url;
                this.method = init.method || input.method;
                this.headers = new Headers(init.headers || input.headers);
                // Body handling for clone
                if (init.body !== undefined) {
                    this._initBody(init.body);
                } else if (input.body && !input.bodyUsed) {
                    this._initBody(input.body);
                } else {
                    this.body = null;
                }
            } else {
                // URL string
                this.url = String(input);
                this.method = (init.method || 'GET').toUpperCase();
                this.headers = new Headers(init.headers);
                this._initBody(init.body);
            }

            this.bodyUsed = false;

            // Additional properties (simplified)
            this.mode = init.mode || 'cors';
            this.credentials = init.credentials || 'same-origin';
            this.cache = init.cache || 'default';
            this.redirect = init.redirect || 'follow';
            this.referrer = init.referrer || 'about:client';
            this.integrity = init.integrity || '';
        }

        _initBody(body) {
            if (body instanceof ReadableStream) {
                this.body = body;
            } else if (body instanceof Uint8Array || body instanceof ArrayBuffer) {
                const bytes = body instanceof Uint8Array ? body : new Uint8Array(body);
                this.body = new ReadableStream({
                    start(controller) {
                        controller.enqueue(bytes);
                        controller.close();
                    }
                });
            } else if (body === null || body === undefined) {
                this.body = null;
            } else {
                // String or other
                const encoder = new TextEncoder();
                const bytes = encoder.encode(String(body));
                this.body = new ReadableStream({
                    start(controller) {
                        controller.enqueue(bytes);
                        controller.close();
                    }
                });
            }
        }

        async text() {
            if (this.bodyUsed) {
                throw new TypeError('Body has already been consumed');
            }
            this.bodyUsed = true;

            if (!this.body) {
                return '';
            }

            const reader = this.body.getReader();
            const chunks = [];

            try {
                while (true) {
                    const { done, value } = await reader.read();
                    if (done) break;
                    chunks.push(value);
                }
            } finally {
                reader.releaseLock();
            }

            const totalLength = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
            const result = new Uint8Array(totalLength);
            let offset = 0;
            for (const chunk of chunks) {
                result.set(chunk, offset);
                offset += chunk.length;
            }

            const decoder = new TextDecoder();
            return decoder.decode(result);
        }

        async json() {
            const text = await this.text();
            return JSON.parse(text);
        }

        async arrayBuffer() {
            if (this.bodyUsed) {
                throw new TypeError('Body has already been consumed');
            }
            this.bodyUsed = true;

            if (!this.body) {
                return new ArrayBuffer(0);
            }

            const reader = this.body.getReader();
            const chunks = [];

            try {
                while (true) {
                    const { done, value } = await reader.read();
                    if (done) break;
                    chunks.push(value);
                }
            } finally {
                reader.releaseLock();
            }

            const totalLength = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
            const result = new Uint8Array(totalLength);
            let offset = 0;
            for (const chunk of chunks) {
                result.set(chunk, offset);
                offset += chunk.length;
            }

            return result.buffer;
        }

        clone() {
            if (this.bodyUsed) {
                throw new TypeError('Cannot clone a Request whose body has been consumed');
            }

            // For simplicity, create a new Request with same properties
            // Note: proper implementation would tee() the body stream
            return new Request(this.url, {
                method: this.method,
                headers: this.headers,
                body: this.body,
                mode: this.mode,
                credentials: this.credentials,
                cache: this.cache,
                redirect: this.redirect,
                referrer: this.referrer,
                integrity: this.integrity
            });
        }
    };
"#;

use rusty_jsc::JSContext;

/// Setup Request class
pub fn setup_request(context: &mut JSContext) {
    context
        .evaluate_script(REQUEST_JS, 1)
        .expect("Failed to setup Request class");
}
