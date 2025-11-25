use rusty_jsc::JSContext;

/// Setup global Response class with streaming body support
pub fn setup_response(context: &mut JSContext) {
    let code = r#"
        globalThis.Response = class Response {
            constructor(body, init) {
                init = init || {};
                this.status = init.status || 200;
                this.statusText = init.statusText || '';
                this.ok = this.status >= 200 && this.status < 300;
                this.bodyUsed = false;
                this._nativeStreamId = null;  // Will be set if body is a native stream

                // Convert headers to Headers instance if available
                if (typeof Headers !== 'undefined') {
                    if (init.headers instanceof Headers) {
                        this.headers = init.headers;
                    } else {
                        this.headers = new Headers(init.headers);
                    }
                } else {
                    // Fallback to plain object
                    this.headers = init.headers || {};
                }

                // Support different body types
                if (body instanceof ReadableStream) {
                    // Already a stream - use it directly
                    this.body = body;
                    // Check if this is a native stream (from fetch)
                    if (body._nativeStreamId !== undefined) {
                        this._nativeStreamId = body._nativeStreamId;
                    }
                } else if (body instanceof Uint8Array || body instanceof ArrayBuffer) {
                    // Binary data - wrap in a stream
                    const bytes = body instanceof Uint8Array ? body : new Uint8Array(body);
                    this.body = new ReadableStream({
                        start(controller) {
                            controller.enqueue(bytes);
                            controller.close();
                        }
                    });
                } else if (body === null || body === undefined) {
                    // Empty body
                    this.body = null;
                } else {
                    // String or other - convert to bytes and wrap in stream
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

            // text() method - read stream and decode to string
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

                // Concatenate all chunks
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

            // arrayBuffer() method - read stream and return buffer
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

                // Concatenate all chunks
                const totalLength = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
                const result = new Uint8Array(totalLength);
                let offset = 0;
                for (const chunk of chunks) {
                    result.set(chunk, offset);
                    offset += chunk.length;
                }

                return result.buffer;
            }

            // bytes() method - read stream and return Uint8Array
            async bytes() {
                if (this.bodyUsed) {
                    throw new TypeError('Body has already been consumed');
                }
                this.bodyUsed = true;

                if (!this.body) {
                    return new Uint8Array(0);
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

                // Concatenate all chunks
                const totalLength = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
                const result = new Uint8Array(totalLength);
                let offset = 0;
                for (const chunk of chunks) {
                    result.set(chunk, offset);
                    offset += chunk.length;
                }

                return result;
            }

            // json() method - decode and parse
            async json() {
                const text = await this.text();
                return JSON.parse(text);
            }

            // Internal method to synchronously get raw body bytes
            // Used by the Rust runtime to extract response body
            _getRawBody() {
                if (!this.body || !this.body._controller) {
                    return new Uint8Array(0);
                }

                const queue = this.body._controller._queue;
                if (!queue || queue.length === 0) {
                    return new Uint8Array(0);
                }

                // Concatenate all chunks in the queue
                const chunks = [];
                for (const item of queue) {
                    if (item.type === 'chunk' && item.value) {
                        chunks.push(item.value);
                    }
                }

                if (chunks.length === 0) {
                    return new Uint8Array(0);
                }

                // Single chunk - return directly
                if (chunks.length === 1) {
                    return chunks[0];
                }

                // Multiple chunks - concatenate
                const totalLength = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
                const result = new Uint8Array(totalLength);
                let offset = 0;
                for (const chunk of chunks) {
                    result.set(chunk, offset);
                    offset += chunk.length;
                }

                return result;
            }

            // Clone the response
            clone() {
                if (this.bodyUsed) {
                    throw new TypeError('Cannot clone a Response whose body has been consumed');
                }

                // For simplicity, get raw body and create new response
                // Note: proper implementation would tee() the body stream
                const bodyBytes = this._getRawBody();
                return new Response(bodyBytes, {
                    status: this.status,
                    statusText: this.statusText,
                    headers: this.headers
                });
            }

            // Static methods
            static error() {
                const response = new Response(null, { status: 0, statusText: '' });
                response.type = 'error';
                return response;
            }

            static redirect(url, status = 302) {
                if (![301, 302, 303, 307, 308].includes(status)) {
                    throw new RangeError('Invalid status code for redirect');
                }
                return new Response(null, {
                    status: status,
                    headers: { 'Location': url }
                });
            }

            static json(data, init) {
                init = init || {};
                const headers = new Headers(init.headers);
                if (!headers.has('content-type')) {
                    headers.set('content-type', 'application/json');
                }
                return new Response(JSON.stringify(data), {
                    ...init,
                    headers: headers
                });
            }
        };
    "#;

    context
        .evaluate_script(code, 1)
        .expect("Failed to setup Response");
}
