use rusty_jsc::JSContext;

/// Setup URL and URLSearchParams APIs
pub fn setup_url_api(context: &mut JSContext) {
    // Minimal URL implementation for parsing
    let url_impl = r#"
        globalThis.URL = class URL {
            constructor(url, base) {
                // Simple URL parsing
                this.href = base ? new URL(base).origin + url : url;

                // Parse the URL
                const match = this.href.match(/^(([^:/?#]+):)?(\/\/([^/?#]*))?([^?#]*)(\?([^#]*))?(#(.*))?/);

                this.protocol = match[2] ? match[2] + ':' : '';
                this.host = match[4] || '';
                this.pathname = match[5] || '/';
                this.search = match[6] || '';
                this.hash = match[8] || '';

                // Parse hostname and port
                const hostMatch = this.host.match(/^([^:]+)(:(.+))?$/);
                this.hostname = hostMatch ? hostMatch[1] : '';
                this.port = hostMatch && hostMatch[3] ? hostMatch[3] : '';

                // Origin
                this.origin = this.protocol + '//' + this.host;

                // SearchParams
                this.searchParams = new URLSearchParams(this.search.substring(1));
            }

            toString() {
                return this.href;
            }
        };

        globalThis.URLSearchParams = class URLSearchParams {
            constructor(init) {
                this._params = new Map();

                if (typeof init === 'string') {
                    // Parse query string
                    init.split('&').forEach(pair => {
                        if (pair) {
                            const [key, value] = pair.split('=').map(decodeURIComponent);
                            this._params.set(key, value || '');
                        }
                    });
                } else if (init) {
                    // Parse from object
                    Object.entries(init).forEach(([key, value]) => {
                        this._params.set(key, String(value));
                    });
                }
            }

            get(name) {
                return this._params.get(name) || null;
            }

            has(name) {
                return this._params.has(name);
            }

            set(name, value) {
                this._params.set(name, String(value));
            }

            delete(name) {
                this._params.delete(name);
            }

            toString() {
                const parts = [];
                this._params.forEach((value, key) => {
                    parts.push(encodeURIComponent(key) + '=' + encodeURIComponent(value));
                });
                return parts.join('&');
            }
        };
    "#;

    context.evaluate_script(url_impl, 1).unwrap();
}
