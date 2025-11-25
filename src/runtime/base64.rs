use rusty_jsc::JSContext;

/// Setup atob/btoa (Base64 encoding/decoding)
pub fn setup_base64(context: &mut JSContext) {
    let code = r#"
        // Base64 encoding/decoding (atob/btoa)
        const BASE64_CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

        globalThis.btoa = function(str) {
            const bytes = typeof str === 'string'
                ? new TextEncoder().encode(str)
                : new Uint8Array(str);

            let result = '';
            const len = bytes.length;

            for (let i = 0; i < len; i += 3) {
                const b1 = bytes[i];
                const b2 = i + 1 < len ? bytes[i + 1] : 0;
                const b3 = i + 2 < len ? bytes[i + 2] : 0;

                result += BASE64_CHARS[b1 >> 2];
                result += BASE64_CHARS[((b1 & 3) << 4) | (b2 >> 4)];
                result += i + 1 < len ? BASE64_CHARS[((b2 & 15) << 2) | (b3 >> 6)] : '=';
                result += i + 2 < len ? BASE64_CHARS[b3 & 63] : '=';
            }

            return result;
        };

        globalThis.atob = function(base64) {
            // Remove whitespace and validate
            base64 = base64.replace(/\s/g, '');

            const len = base64.length;
            if (len % 4 !== 0) {
                throw new Error('Invalid base64 string');
            }

            // Calculate output length
            let outputLen = (len / 4) * 3;
            if (base64[len - 1] === '=') outputLen--;
            if (base64[len - 2] === '=') outputLen--;

            const bytes = new Uint8Array(outputLen);
            let p = 0;

            for (let i = 0; i < len; i += 4) {
                const c1 = BASE64_CHARS.indexOf(base64[i]);
                const c2 = BASE64_CHARS.indexOf(base64[i + 1]);
                const c3 = base64[i + 2] === '=' ? 0 : BASE64_CHARS.indexOf(base64[i + 2]);
                const c4 = base64[i + 3] === '=' ? 0 : BASE64_CHARS.indexOf(base64[i + 3]);

                if (c1 === -1 || c2 === -1 || (base64[i + 2] !== '=' && c3 === -1) || (base64[i + 3] !== '=' && c4 === -1)) {
                    throw new Error('Invalid base64 character');
                }

                bytes[p++] = (c1 << 2) | (c2 >> 4);
                if (base64[i + 2] !== '=') bytes[p++] = ((c2 & 15) << 4) | (c3 >> 2);
                if (base64[i + 3] !== '=') bytes[p++] = ((c3 & 3) << 6) | c4;
            }

            return new TextDecoder().decode(bytes);
        };
    "#;

    context
        .evaluate_script(code, 1)
        .expect("Failed to setup atob/btoa");
}
