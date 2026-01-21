use openworkers_core::{Event, HttpMethod, HttpRequest, RequestBody, Script};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;

/// Test crypto.getRandomValues
#[tokio::test]
async fn test_get_random_values() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const array = new Uint8Array(16);
            crypto.getRandomValues(array);

            // Check that at least some bytes are non-zero
            const hasNonZero = array.some(b => b !== 0);

            event.respondWith(new Response(hasNonZero ? 'OK' : 'FAIL'));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

/// Test crypto.randomUUID
#[tokio::test]
async fn test_random_uuid() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const uuid = crypto.randomUUID();

            // UUID v4 format: xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx
            const uuidRegex = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
            const isValid = uuidRegex.test(uuid);

            event.respondWith(new Response(isValid ? 'OK' : 'FAIL: ' + uuid));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

/// Test crypto.subtle.digest with SHA-256
#[tokio::test]
async fn test_digest_sha256() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const data = new TextEncoder().encode('hello world');
            const hash = await crypto.subtle.digest('SHA-256', data);

            // SHA-256 of "hello world" is known
            const hashArray = new Uint8Array(hash);
            const hashHex = Array.from(hashArray)
                .map(b => b.toString(16).padStart(2, '0'))
                .join('');

            // Expected: b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
            const expected = 'b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9';

            event.respondWith(new Response(hashHex === expected ? 'OK' : 'FAIL: ' + hashHex));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

/// Test crypto.subtle.digest with SHA-512
#[tokio::test]
async fn test_digest_sha512() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const data = new TextEncoder().encode('test');
            const hash = await crypto.subtle.digest('SHA-512', data);

            // SHA-512 produces 64 bytes
            const hashArray = new Uint8Array(hash);

            event.respondWith(new Response(hashArray.length === 64 ? 'OK' : 'FAIL'));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

/// Test HMAC sign and verify
#[tokio::test]
async fn test_hmac_sign_verify() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            // Create a key
            const keyData = new TextEncoder().encode('my-secret-key');
            const key = await crypto.subtle.importKey(
                'raw',
                keyData,
                { name: 'HMAC', hash: 'SHA-256' },
                false,
                ['sign', 'verify']
            );

            // Sign some data
            const data = new TextEncoder().encode('hello world');
            const signature = await crypto.subtle.sign('HMAC', key, data);

            // Verify the signature
            const isValid = await crypto.subtle.verify('HMAC', key, signature, data);

            // Try to verify with wrong data
            const wrongData = new TextEncoder().encode('wrong data');
            const isInvalid = await crypto.subtle.verify('HMAC', key, signature, wrongData);

            const result = isValid && !isInvalid ? 'OK' : 'FAIL';
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

/// Test ECDSA P-256 key generation, sign and verify
#[tokio::test]
async fn test_ecdsa_sign_verify() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            // Generate an ECDSA P-256 key pair
            const keyPair = await crypto.subtle.generateKey(
                { name: 'ECDSA', namedCurve: 'P-256' },
                true,
                ['sign', 'verify']
            );

            // Sign some data
            const data = new TextEncoder().encode('hello world');
            const signature = await crypto.subtle.sign(
                { name: 'ECDSA', hash: 'SHA-256' },
                keyPair.privateKey,
                data
            );

            // Verify with public key
            const isValid = await crypto.subtle.verify(
                { name: 'ECDSA', hash: 'SHA-256' },
                keyPair.publicKey,
                signature,
                data
            );

            // Try to verify with wrong data
            const wrongData = new TextEncoder().encode('wrong data');
            const isInvalid = await crypto.subtle.verify(
                { name: 'ECDSA', hash: 'SHA-256' },
                keyPair.publicKey,
                signature,
                wrongData
            );

            // ECDSA P-256 signature is 64 bytes (r||s, each 32 bytes)
            const sigLen = new Uint8Array(signature).length;

            const result = isValid && !isInvalid && sigLen === 64 ? 'OK' : `FAIL: isValid=${isValid}, isInvalid=${isInvalid}, sigLen=${sigLen}`;
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

/// Test ECDSA verify with private key (should use embedded public key)
#[tokio::test]
async fn test_ecdsa_verify_with_private_key() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            // Generate key pair
            const keyPair = await crypto.subtle.generateKey(
                { name: 'ECDSA', namedCurve: 'P-256' },
                true,
                ['sign', 'verify']
            );

            // Sign data
            const data = new TextEncoder().encode('test message');
            const signature = await crypto.subtle.sign(
                { name: 'ECDSA', hash: 'SHA-256' },
                keyPair.privateKey,
                data
            );

            // Verify with private key (should work, uses embedded public key)
            const isValid = await crypto.subtle.verify(
                { name: 'ECDSA', hash: 'SHA-256' },
                keyPair.privateKey,
                signature,
                data
            );

            event.respondWith(new Response(isValid ? 'OK' : 'FAIL'));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

/// Test RSA PKCS#1 v1.5 sign and verify
#[tokio::test]
async fn test_rsa_sign_verify() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            try {
                // Base64 decoder that returns Uint8Array
                function base64ToBytes(base64) {
                    const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
                    const len = base64.length;
                    let bufferLength = len * 0.75;
                    if (base64[len - 1] === '=') bufferLength--;
                    if (base64[len - 2] === '=') bufferLength--;

                    const bytes = new Uint8Array(Math.floor(bufferLength));
                    let p = 0;

                    for (let i = 0; i < len; i += 4) {
                        const e1 = chars.indexOf(base64[i]);
                        const e2 = chars.indexOf(base64[i + 1]);
                        const e3 = chars.indexOf(base64[i + 2]);
                        const e4 = chars.indexOf(base64[i + 3]);

                        bytes[p++] = (e1 << 2) | (e2 >> 4);
                        if (e3 !== -1 && base64[i + 2] !== '=') {
                            bytes[p++] = ((e2 & 15) << 4) | (e3 >> 2);
                        }
                        if (e4 !== -1 && base64[i + 3] !== '=') {
                            bytes[p++] = ((e3 & 3) << 6) | e4;
                        }
                    }
                    return bytes;
                }

                // Base64 encoded RSA keys (2048-bit)
                const privateKeyBase64 = 'MIIEpAIBAAKCAQEA5EmDGTHoMj6bosn6lbZMJkZNnDlfoon7eMBrVQYSkQDLZCnJHDAxAD8ODlIWlRHDD9NWqyEBdTGqlUDTrjKvLBzktSMWeIG0TrXVQ0Yw3Ibu8EvSn8tGVEq/Epa05uNh7JGVjxmIRVyGn6ic9b1S85JzfcSJgUoxSvW0KmTOh/TaaHdAkGS/4wpdfjSexogWapyKNms17jHehmtkUq0Vhh4YYr8t72bb+FJtHqwsEYbC3jXXEQ+u6zCmc9fDuAvbv5kvjglBZu0aEGap5fmbqSWexWqJcdvln7TMQ2A6b1fmZ1t76+WtKH7WwGf4SGkJ2PLFxCZaJ8oE0Ci+Rm/amwIDAQABAoIBABBogj5A2o4l9tzMBLFXEYEcw35Ll2ag4UzME8rgLVxzwKq54CUhB5yba6C24L2lMa6FA7E4JZktUTP6HVzjcrjKeNvWIkrWE8YmhqYXuPJY1nq6EHEA1NTBLJui7my8AjFVQ3kuHh/SJzD5lxKIoZo1OAzdn/6FfSaEo4b6iOe3nGj2q00WUf4t5OjQyWkgZHb3D+QFimnrw0q0ct/N28MxiHohJs+8NgDhDnjthF1fwi5mpso9mm+ysw2/ss5W1y6mczWcEwXvTh0svD6BkdGHfdpbkaXguHFCyFk1WG80MYiq61yZOwPMj2GFh/o3dPdIo5x3ScKBzDen2zuevLkCgYEA+N30zLXWM67jpqxFTokbEiImmUiLHGPx4CtCu1Cf3MNWz7W2p8/eCi3vtL8dCeD687yuHDPcft6KH88jXHriyTaK3zez8BTetmGNM+3YM1QmFynD1qYuqaDyobZBFwpxka902SQFcWAIDsimaJeNsVd2Kxr2lb3AYZyu66vQAtkCgYEA6tSNeHWS+PqF0OUivYI2Vsn+moplxNfEElSK6ifrK39YaEv9hZzwLIR3Iq0cxbKQWBNvssWzaLd0Z5ZKBEWFmLDNph7Giq7V1spUc6V6tWrbGL+92Yw0+ZWjx83InFAT+B6Cjgvptfrd0AipphhrAFC0c3iiIKbSPv0EOPec+JMCgYEAx0rneO+9A1JwV87pCYVeOl1Cz8l6LVgUIFJEdECSZHXBlUCNb0FVLI2wweux03FpRbq5KziUwLxxnBuC09JMvpmBCFRRMldkKmVgcE9trV0by7zUaZZXE9whsUKESXFBlUsOpbzk5u/iRASGzodfHr9NkCNdiHiWERUqNuw1/bECgYASDVDqx68KsMeErXikNNRUi6ak3qrAHQ4XkqQzJ+puJ5X2PpE4qj3UTkKSSdiCYh2yh5v4lDYcgK3UILuD5IxGlqDYelks5A/QOTGQylHKjHJXTrYbeSnBXf1/KJSZX5aJZl8G6GeI88YFbgUMnafsGEgm8EkWVXyoFu8yKebJPQKBgQDsltWFU9zmXXA6mMaKi5A7J7Va3s74pEqlyQk+Xb0iRcZLKCIdB3MepaIPXi0QPjRwXY6vIVIV2AvTToup1c4pZKH98YM/HFZfLgQsNw0YGW39VzyR4i39j44AvAmLB0y8x8GKD7NUk8cVJGLL+R5qyRe2LGOJtHb4UoBsmTCIWg==';
                const publicKeyBase64 = 'MIIBCgKCAQEA5EmDGTHoMj6bosn6lbZMJkZNnDlfoon7eMBrVQYSkQDLZCnJHDAxAD8ODlIWlRHDD9NWqyEBdTGqlUDTrjKvLBzktSMWeIG0TrXVQ0Yw3Ibu8EvSn8tGVEq/Epa05uNh7JGVjxmIRVyGn6ic9b1S85JzfcSJgUoxSvW0KmTOh/TaaHdAkGS/4wpdfjSexogWapyKNms17jHehmtkUq0Vhh4YYr8t72bb+FJtHqwsEYbC3jXXEQ+u6zCmc9fDuAvbv5kvjglBZu0aEGap5fmbqSWexWqJcdvln7TMQ2A6b1fmZ1t76+WtKH7WwGf4SGkJ2PLFxCZaJ8oE0Ci+Rm/amwIDAQAB';

                const privateKeyData = base64ToBytes(privateKeyBase64);
                const publicKeyData = base64ToBytes(publicKeyBase64);

                // Import keys
                const privateKey = await crypto.subtle.importKey(
                    'pkcs8',
                    privateKeyData,
                    { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' },
                    false,
                    ['sign']
                );

                const publicKey = await crypto.subtle.importKey(
                    'spki',
                    publicKeyData,
                    { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' },
                    false,
                    ['verify']
                );

                // Sign data
                const data = new TextEncoder().encode('hello world');
                const signature = await crypto.subtle.sign(
                    'RSASSA-PKCS1-v1_5',
                    privateKey,
                    data
                );

                // Verify signature
                const isValid = await crypto.subtle.verify(
                    'RSASSA-PKCS1-v1_5',
                    publicKey,
                    signature,
                    data
                );

                // Verify with wrong data fails
                const wrongData = new TextEncoder().encode('wrong data');
                const isInvalid = await crypto.subtle.verify(
                    'RSASSA-PKCS1-v1_5',
                    publicKey,
                    signature,
                    wrongData
                );

                // RSA-2048 signature is 256 bytes
                const sigLen = new Uint8Array(signature).length;

                const result = isValid && !isInvalid && sigLen === 256 ? 'OK' : `FAIL: isValid=${isValid}, isInvalid=${isInvalid}, sigLen=${sigLen}`;
                event.respondWith(new Response(result));
            } catch (e) {
                event.respondWith(new Response('ERROR: ' + e.message));
            }
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

/// Test HMAC with different hash algorithms
#[tokio::test]
async fn test_hmac_different_algorithms() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const keyData = new TextEncoder().encode('secret');
            const data = new TextEncoder().encode('message');

            // Test SHA-256
            const key256 = await crypto.subtle.importKey(
                'raw', keyData, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']
            );
            const sig256 = await crypto.subtle.sign('HMAC', key256, data);
            const len256 = new Uint8Array(sig256).length;

            // Test SHA-384
            const key384 = await crypto.subtle.importKey(
                'raw', keyData, { name: 'HMAC', hash: 'SHA-384' }, false, ['sign']
            );
            const sig384 = await crypto.subtle.sign('HMAC', key384, data);
            const len384 = new Uint8Array(sig384).length;

            // Test SHA-512
            const key512 = await crypto.subtle.importKey(
                'raw', keyData, { name: 'HMAC', hash: 'SHA-512' }, false, ['sign']
            );
            const sig512 = await crypto.subtle.sign('HMAC', key512, data);
            const len512 = new Uint8Array(sig512).length;

            // SHA-256 = 32 bytes, SHA-384 = 48 bytes, SHA-512 = 64 bytes
            const result = (len256 === 32 && len384 === 48 && len512 === 64) ? 'OK' : 'FAIL';
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}
