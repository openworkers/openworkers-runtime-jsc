use ring::{digest, hmac, rand, rsa, signature, signature::KeyPair};
use rusty_jsc::JSContext;

/// Setup crypto global object with getRandomValues, randomUUID, and subtle
pub fn setup_crypto(context: &mut JSContext) {
    // Create __nativeGetRandomValues function
    let get_random_values_fn = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.is_empty() {
                return Err(JSValue::string(
                    &ctx,
                    "getRandomValues requires an argument",
                ));
            }

            let array = match args[0].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "Argument must be a TypedArray")),
            };

            // Get the typed array buffer and fill with random bytes
            let bytes = unsafe {
                match array.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice,
                    Err(_) => return Err(JSValue::string(&ctx, "Argument must be a TypedArray")),
                }
            };

            // Fill with random bytes using ring
            let rng = rand::SystemRandom::new();

            if rand::SecureRandom::fill(&rng, bytes).is_err() {
                return Err(JSValue::string(&ctx, "Failed to generate random bytes"));
            }

            // Return the same array
            Ok(args[0].clone())
        }
    );

    // Create __nativeRandomUUID function
    let random_uuid_fn = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, _args: &[JSValue]| {
            let uuid = uuid::Uuid::new_v4().to_string();
            Ok(JSValue::string(&ctx, uuid.as_str()))
        }
    );

    // Create __nativeDigest(algorithm, data) -> Uint8Array
    let digest_fn = rusty_jsc::callback_closure!(
        context,
        move |mut ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 2 {
                return Err(JSValue::string(&ctx, "digest requires algorithm and data"));
            }

            // Get algorithm name
            let algo = match args[0].to_js_string(&ctx) {
                Ok(s) => s.to_string().to_uppercase(),
                Err(_) => return Err(JSValue::string(&ctx, "Algorithm must be a string")),
            };

            // Get data as Uint8Array
            let data_obj = match args[1].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "Data must be a Uint8Array")),
            };

            let data = unsafe {
                match data_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Err(JSValue::string(&ctx, "Data must be a Uint8Array")),
                }
            };

            // Select algorithm
            let algorithm = match algo.as_str() {
                "SHA-1" => &digest::SHA1_FOR_LEGACY_USE_ONLY,
                "SHA-256" => &digest::SHA256,
                "SHA-384" => &digest::SHA384,
                "SHA-512" => &digest::SHA512,
                _ => return Err(JSValue::string(&ctx, "Unsupported algorithm")),
            };

            // Compute digest
            let result = digest::digest(algorithm, &data);
            let result_bytes = result.as_ref();

            // Create Uint8Array with result by converting to JSON array and back
            let json_array: Vec<u8> = result_bytes.to_vec();
            let json_str = serde_json::to_string(&json_array).unwrap();
            let script = format!("new Uint8Array({}).buffer", json_str);

            match ctx.evaluate_script(&script, 1) {
                Ok(buffer) => Ok(buffer),
                Err(_) => Err(JSValue::string(&ctx, "Failed to create ArrayBuffer")),
            }
        }
    );

    // Create __nativeHmacSign(algorithm, keyData, data) -> Uint8Array
    let hmac_sign_fn = rusty_jsc::callback_closure!(
        context,
        move |mut ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 3 {
                return Err(JSValue::string(
                    &ctx,
                    "hmacSign requires algorithm, keyData, and data",
                ));
            }

            // Get algorithm name
            let algo = match args[0].to_js_string(&ctx) {
                Ok(s) => s.to_string().to_uppercase(),
                Err(_) => return Err(JSValue::string(&ctx, "Algorithm must be a string")),
            };

            // Get key data
            let key_obj = match args[1].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "Key must be a Uint8Array")),
            };

            let key_data = unsafe {
                match key_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Err(JSValue::string(&ctx, "Key must be a Uint8Array")),
                }
            };

            // Get data
            let data_obj = match args[2].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "Data must be a Uint8Array")),
            };

            let data = unsafe {
                match data_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Err(JSValue::string(&ctx, "Data must be a Uint8Array")),
                }
            };

            // Select algorithm
            let algorithm = match algo.as_str() {
                "SHA-1" => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
                "SHA-256" => hmac::HMAC_SHA256,
                "SHA-384" => hmac::HMAC_SHA384,
                "SHA-512" => hmac::HMAC_SHA512,
                _ => return Err(JSValue::string(&ctx, "Unsupported HMAC algorithm")),
            };

            // Sign
            let key = hmac::Key::new(algorithm, &key_data);
            let tag = hmac::sign(&key, &data);
            let result_bytes = tag.as_ref();

            // Create Uint8Array with result
            let json_array: Vec<u8> = result_bytes.to_vec();
            let json_str = serde_json::to_string(&json_array).unwrap();
            let script = format!("new Uint8Array({}).buffer", json_str);

            match ctx.evaluate_script(&script, 1) {
                Ok(buffer) => Ok(buffer),
                Err(_) => Err(JSValue::string(&ctx, "Failed to create ArrayBuffer")),
            }
        }
    );

    // Create __nativeHmacVerify(algorithm, keyData, signature, data) -> boolean
    let hmac_verify_fn = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 4 {
                return Err(JSValue::string(
                    &ctx,
                    "hmacVerify requires algorithm, keyData, signature, and data",
                ));
            }

            // Get algorithm name
            let algo = match args[0].to_js_string(&ctx) {
                Ok(s) => s.to_string().to_uppercase(),
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            // Get key data
            let key_obj = match args[1].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            let key_data = unsafe {
                match key_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Ok(JSValue::boolean(&ctx, false)),
                }
            };

            // Get signature
            let sig_obj = match args[2].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            let signature = unsafe {
                match sig_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Ok(JSValue::boolean(&ctx, false)),
                }
            };

            // Get data
            let data_obj = match args[3].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            let data = unsafe {
                match data_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Ok(JSValue::boolean(&ctx, false)),
                }
            };

            // Select algorithm
            let algorithm = match algo.as_str() {
                "SHA-1" => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
                "SHA-256" => hmac::HMAC_SHA256,
                "SHA-384" => hmac::HMAC_SHA384,
                "SHA-512" => hmac::HMAC_SHA512,
                _ => return Ok(JSValue::boolean(&ctx, false)),
            };

            // Verify
            let key = hmac::Key::new(algorithm, &key_data);
            let is_valid = hmac::verify(&key, &data, &signature).is_ok();

            Ok(JSValue::boolean(&ctx, is_valid))
        }
    );

    // Create __nativeEcdsaGenerateKey() -> { privateKey: ArrayBuffer, publicKey: ArrayBuffer }
    let ecdsa_generate_fn = rusty_jsc::callback_closure!(
        context,
        move |mut ctx: JSContext, _func: JSObject, _this: JSObject, _args: &[JSValue]| {
            let rng = rand::SystemRandom::new();

            // Generate ECDSA P-256 key pair
            let pkcs8_bytes = match signature::EcdsaKeyPair::generate_pkcs8(
                &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                &rng,
            ) {
                Ok(bytes) => bytes,
                Err(_) => return Err(JSValue::string(&ctx, "Key generation failed")),
            };

            // Parse the key pair to get the public key
            let key_pair = match signature::EcdsaKeyPair::from_pkcs8(
                &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                pkcs8_bytes.as_ref(),
                &rng,
            ) {
                Ok(kp) => kp,
                Err(_) => return Err(JSValue::string(&ctx, "Failed to parse key pair")),
            };

            let public_key_bytes = key_pair.public_key().as_ref();

            // Create result object with privateKey and publicKey as JSON arrays
            let private_json = serde_json::to_string(&pkcs8_bytes.as_ref().to_vec()).unwrap();
            let public_json = serde_json::to_string(&public_key_bytes.to_vec()).unwrap();

            let script = format!(
                "({{ privateKey: new Uint8Array({}).buffer, publicKey: new Uint8Array({}).buffer }})",
                private_json, public_json
            );

            match ctx.evaluate_script(&script, 1) {
                Ok(result) => Ok(result),
                Err(_) => Err(JSValue::string(&ctx, "Failed to create key pair object")),
            }
        }
    );

    // Create __nativeEcdsaSign(privateKeyPkcs8, data) -> ArrayBuffer
    let ecdsa_sign_fn = rusty_jsc::callback_closure!(
        context,
        move |mut ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 2 {
                return Err(JSValue::string(
                    &ctx,
                    "ecdsaSign requires privateKey and data",
                ));
            }

            let key_obj = match args[0].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "Private key must be a Uint8Array")),
            };

            let private_key_data = unsafe {
                match key_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => {
                        return Err(JSValue::string(&ctx, "Private key must be a Uint8Array"));
                    }
                }
            };

            let data_obj = match args[1].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "Data must be a Uint8Array")),
            };

            let data = unsafe {
                match data_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Err(JSValue::string(&ctx, "Data must be a Uint8Array")),
                }
            };

            let rng = rand::SystemRandom::new();

            // Load the key pair from PKCS#8
            let key_pair = match signature::EcdsaKeyPair::from_pkcs8(
                &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                &private_key_data,
                &rng,
            ) {
                Ok(kp) => kp,
                Err(_) => return Err(JSValue::string(&ctx, "Invalid private key")),
            };

            // Sign the data
            let sig = match key_pair.sign(&rng, &data) {
                Ok(s) => s,
                Err(_) => return Err(JSValue::string(&ctx, "Signing failed")),
            };

            let json_array = serde_json::to_string(&sig.as_ref().to_vec()).unwrap();
            let script = format!("new Uint8Array({}).buffer", json_array);

            match ctx.evaluate_script(&script, 1) {
                Ok(buffer) => Ok(buffer),
                Err(_) => Err(JSValue::string(&ctx, "Failed to create signature buffer")),
            }
        }
    );

    // Create __nativeEcdsaVerify(publicKey, signature, data) -> boolean
    let ecdsa_verify_fn = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 3 {
                return Ok(JSValue::boolean(&ctx, false));
            }

            let public_key_obj = match args[0].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            let public_key_data = unsafe {
                match public_key_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Ok(JSValue::boolean(&ctx, false)),
                }
            };

            let sig_obj = match args[1].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            let sig_data = unsafe {
                match sig_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Ok(JSValue::boolean(&ctx, false)),
                }
            };

            let data_obj = match args[2].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            let data = unsafe {
                match data_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Ok(JSValue::boolean(&ctx, false)),
                }
            };

            // Verify using UnparsedPublicKey
            let public_key = signature::UnparsedPublicKey::new(
                &signature::ECDSA_P256_SHA256_FIXED,
                &public_key_data,
            );

            let is_valid = public_key.verify(&data, &sig_data).is_ok();
            Ok(JSValue::boolean(&ctx, is_valid))
        }
    );

    // Create __nativeRsaSign(hashAlgo, privateKeyDer, data) -> ArrayBuffer
    let rsa_sign_fn = rusty_jsc::callback_closure!(
        context,
        move |mut ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 3 {
                return Err(JSValue::string(
                    &ctx,
                    "rsaSign requires hashAlgo, privateKey, and data",
                ));
            }

            let hash_algo = match args[0].to_js_string(&ctx) {
                Ok(s) => s.to_string().to_uppercase(),
                Err(_) => return Err(JSValue::string(&ctx, "Hash algorithm must be a string")),
            };

            let key_obj = match args[1].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "Private key must be a Uint8Array")),
            };

            let private_key_data = unsafe {
                match key_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => {
                        return Err(JSValue::string(&ctx, "Private key must be a Uint8Array"));
                    }
                }
            };

            let data_obj = match args[2].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "Data must be a Uint8Array")),
            };

            let data = unsafe {
                match data_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Err(JSValue::string(&ctx, "Data must be a Uint8Array")),
                }
            };

            // Select padding/encoding based on hash algorithm
            let padding = match hash_algo.as_str() {
                "SHA-256" => &signature::RSA_PKCS1_SHA256,
                "SHA-384" => &signature::RSA_PKCS1_SHA384,
                "SHA-512" => &signature::RSA_PKCS1_SHA512,
                _ => return Err(JSValue::string(&ctx, "Unsupported hash algorithm")),
            };

            // Load RSA key pair from DER
            let key_pair = match rsa::KeyPair::from_der(&private_key_data) {
                Ok(kp) => kp,
                Err(_) => return Err(JSValue::string(&ctx, "Invalid RSA private key")),
            };

            let rng = rand::SystemRandom::new();
            let mut sig = vec![0u8; key_pair.public().modulus_len()];

            match key_pair.sign(padding, &rng, &data, &mut sig) {
                Ok(_) => {
                    let json_array = serde_json::to_string(&sig).unwrap();
                    let script = format!("new Uint8Array({}).buffer", json_array);

                    match ctx.evaluate_script(&script, 1) {
                        Ok(buffer) => Ok(buffer),
                        Err(_) => Err(JSValue::string(&ctx, "Failed to create signature buffer")),
                    }
                }
                Err(_) => Err(JSValue::string(&ctx, "RSA signing failed")),
            }
        }
    );

    // Create __nativeRsaVerify(hashAlgo, publicKeyDer, signature, data) -> boolean
    let rsa_verify_fn = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 4 {
                return Ok(JSValue::boolean(&ctx, false));
            }

            let hash_algo = match args[0].to_js_string(&ctx) {
                Ok(s) => s.to_string().to_uppercase(),
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            let public_key_obj = match args[1].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            let public_key_data = unsafe {
                match public_key_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Ok(JSValue::boolean(&ctx, false)),
                }
            };

            let sig_obj = match args[2].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            let sig_data = unsafe {
                match sig_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Ok(JSValue::boolean(&ctx, false)),
                }
            };

            let data_obj = match args[3].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            let data = unsafe {
                match data_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => slice.to_vec(),
                    Err(_) => return Ok(JSValue::boolean(&ctx, false)),
                }
            };

            // Select verification algorithm based on hash
            let algorithm: &dyn signature::VerificationAlgorithm = match hash_algo.as_str() {
                "SHA-256" => &signature::RSA_PKCS1_2048_8192_SHA256,
                "SHA-384" => &signature::RSA_PKCS1_2048_8192_SHA384,
                "SHA-512" => &signature::RSA_PKCS1_2048_8192_SHA512,
                _ => return Ok(JSValue::boolean(&ctx, false)),
            };

            let public_key = signature::UnparsedPublicKey::new(algorithm, &public_key_data);
            let is_valid = public_key.verify(&data, &sig_data).is_ok();

            Ok(JSValue::boolean(&ctx, is_valid))
        }
    );

    // Add native functions to global
    let mut global = context.get_global_object();
    global
        .set_property(
            context,
            "__nativeGetRandomValues",
            get_random_values_fn.into(),
        )
        .unwrap();
    global
        .set_property(context, "__nativeRandomUUID", random_uuid_fn.into())
        .unwrap();
    global
        .set_property(context, "__nativeDigest", digest_fn.into())
        .unwrap();
    global
        .set_property(context, "__nativeHmacSign", hmac_sign_fn.into())
        .unwrap();
    global
        .set_property(context, "__nativeHmacVerify", hmac_verify_fn.into())
        .unwrap();
    global
        .set_property(
            context,
            "__nativeEcdsaGenerateKey",
            ecdsa_generate_fn.into(),
        )
        .unwrap();
    global
        .set_property(context, "__nativeEcdsaSign", ecdsa_sign_fn.into())
        .unwrap();
    global
        .set_property(context, "__nativeEcdsaVerify", ecdsa_verify_fn.into())
        .unwrap();
    global
        .set_property(context, "__nativeRsaSign", rsa_sign_fn.into())
        .unwrap();
    global
        .set_property(context, "__nativeRsaVerify", rsa_verify_fn.into())
        .unwrap();

    // Create crypto object and subtle with JS wrappers
    let crypto_script = r#"
        // Create crypto object
        globalThis.crypto = {
            getRandomValues: function(typedArray) {
                return __nativeGetRandomValues(typedArray);
            },
            randomUUID: function() {
                return __nativeRandomUUID();
            },
            subtle: {}
        };

        // Simple key storage
        const __cryptoKeys = new Map();
        let __nextKeyId = 1;

        // crypto.subtle.digest(algorithm, data) -> Promise<ArrayBuffer>
        crypto.subtle.digest = function(algorithm, data) {
            return new Promise((resolve, reject) => {
                try {
                    let bytes;
                    if (data instanceof ArrayBuffer) {
                        bytes = new Uint8Array(data);
                    } else if (data instanceof Uint8Array) {
                        bytes = data;
                    } else {
                        reject(new Error('Data must be ArrayBuffer or Uint8Array'));
                        return;
                    }

                    const algoName = typeof algorithm === 'string' ? algorithm : algorithm.name;
                    const result = __nativeDigest(algoName, bytes);

                    if (result) {
                        resolve(result);
                    } else {
                        reject(new Error('Unsupported algorithm: ' + algoName));
                    }
                } catch (e) {
                    reject(e);
                }
            });
        };

        // crypto.subtle.generateKey - ECDSA only
        crypto.subtle.generateKey = function(algorithm, extractable, keyUsages) {
            return new Promise((resolve, reject) => {
                try {
                    const algoName = typeof algorithm === 'string' ? algorithm : algorithm.name;

                    if (algoName === 'ECDSA') {
                        const namedCurve = algorithm.namedCurve || 'P-256';
                        if (namedCurve !== 'P-256') {
                            reject(new Error('Only P-256 curve is supported'));
                            return;
                        }

                        const result = __nativeEcdsaGenerateKey();
                        if (!result) {
                            reject(new Error('Key generation failed'));
                            return;
                        }

                        const keyPair = {
                            privateKey: {
                                type: 'private',
                                extractable: extractable,
                                algorithm: { name: 'ECDSA', namedCurve: 'P-256' },
                                usages: keyUsages.filter(u => u === 'sign'),
                                __keyData: new Uint8Array(result.privateKey),
                                __publicKeyData: new Uint8Array(result.publicKey)
                            },
                            publicKey: {
                                type: 'public',
                                extractable: true,
                                algorithm: { name: 'ECDSA', namedCurve: 'P-256' },
                                usages: keyUsages.filter(u => u === 'verify'),
                                __keyData: new Uint8Array(result.publicKey)
                            }
                        };

                        resolve(keyPair);
                    } else {
                        reject(new Error('Only ECDSA algorithm is supported for generateKey'));
                    }
                } catch (e) {
                    reject(e);
                }
            });
        };

        // crypto.subtle.importKey - HMAC, ECDSA, RSA
        crypto.subtle.importKey = function(format, keyData, algorithm, extractable, keyUsages) {
            return new Promise((resolve, reject) => {
                try {
                    const algoName = typeof algorithm === 'string' ? algorithm : algorithm.name;

                    let keyBytes;
                    if (keyData instanceof ArrayBuffer) {
                        keyBytes = new Uint8Array(keyData);
                    } else if (keyData instanceof Uint8Array) {
                        keyBytes = keyData;
                    } else {
                        reject(new Error('Key data must be ArrayBuffer or Uint8Array'));
                        return;
                    }

                    if (algoName === 'HMAC') {
                        if (format !== 'raw') {
                            reject(new Error('Only "raw" format is supported for HMAC'));
                            return;
                        }

                        const hashName = typeof algorithm === 'object' && algorithm.hash
                            ? (typeof algorithm.hash === 'string' ? algorithm.hash : algorithm.hash.name)
                            : 'SHA-256';

                        const keyId = __nextKeyId++;
                        const key = {
                            type: 'secret',
                            extractable: extractable,
                            algorithm: { name: 'HMAC', hash: { name: hashName } },
                            usages: keyUsages,
                            __keyId: keyId,
                            __keyData: keyBytes
                        };

                        __cryptoKeys.set(keyId, key);
                        resolve(key);
                    } else if (algoName === 'ECDSA') {
                        const namedCurve = algorithm.namedCurve || 'P-256';
                        if (namedCurve !== 'P-256') {
                            reject(new Error('Only P-256 curve is supported'));
                            return;
                        }

                        if (format === 'raw') {
                            // Raw format is for public keys (uncompressed point)
                            const key = {
                                type: 'public',
                                extractable: extractable,
                                algorithm: { name: 'ECDSA', namedCurve: 'P-256' },
                                usages: keyUsages,
                                __keyData: keyBytes
                            };
                            resolve(key);
                        } else if (format === 'pkcs8') {
                            // PKCS#8 format is for private keys
                            const key = {
                                type: 'private',
                                extractable: extractable,
                                algorithm: { name: 'ECDSA', namedCurve: 'P-256' },
                                usages: keyUsages,
                                __keyData: keyBytes
                            };
                            resolve(key);
                        } else {
                            reject(new Error('Only "raw" and "pkcs8" formats are supported for ECDSA'));
                        }
                    } else if (algoName === 'RSASSA-PKCS1-v1_5') {
                        const hashName = typeof algorithm === 'object' && algorithm.hash
                            ? (typeof algorithm.hash === 'string' ? algorithm.hash : algorithm.hash.name)
                            : 'SHA-256';

                        if (format === 'pkcs8') {
                            // PKCS#8 format for private keys
                            const key = {
                                type: 'private',
                                extractable: extractable,
                                algorithm: { name: 'RSASSA-PKCS1-v1_5', hash: { name: hashName } },
                                usages: keyUsages,
                                __keyData: keyBytes
                            };
                            resolve(key);
                        } else if (format === 'spki' || format === 'raw') {
                            // SPKI/raw format for public keys
                            const key = {
                                type: 'public',
                                extractable: extractable,
                                algorithm: { name: 'RSASSA-PKCS1-v1_5', hash: { name: hashName } },
                                usages: keyUsages,
                                __keyData: keyBytes
                            };
                            resolve(key);
                        } else {
                            reject(new Error('Only "pkcs8" and "spki" formats are supported for RSA'));
                        }
                    } else {
                        reject(new Error('Unsupported algorithm: ' + algoName));
                    }
                } catch (e) {
                    reject(e);
                }
            });
        };

        // crypto.subtle.sign - HMAC, ECDSA, RSA
        crypto.subtle.sign = function(algorithm, key, data) {
            return new Promise((resolve, reject) => {
                try {
                    const algoName = typeof algorithm === 'string' ? algorithm : algorithm.name;

                    let dataBytes;
                    if (data instanceof ArrayBuffer) {
                        dataBytes = new Uint8Array(data);
                    } else if (data instanceof Uint8Array) {
                        dataBytes = data;
                    } else {
                        reject(new Error('Data must be ArrayBuffer or Uint8Array'));
                        return;
                    }

                    if (!key.__keyData) {
                        reject(new Error('Invalid key'));
                        return;
                    }

                    if (algoName === 'HMAC') {
                        const hashName = key.algorithm.hash.name;
                        const result = __nativeHmacSign(hashName, key.__keyData, dataBytes);

                        if (result) {
                            resolve(result);
                        } else {
                            reject(new Error('HMAC sign failed'));
                        }
                    } else if (algoName === 'ECDSA') {
                        if (key.type !== 'private' || key.algorithm.name !== 'ECDSA') {
                            reject(new Error('Invalid key for ECDSA signing'));
                            return;
                        }

                        const result = __nativeEcdsaSign(key.__keyData, dataBytes);
                        if (result) {
                            resolve(result);
                        } else {
                            reject(new Error('ECDSA sign failed'));
                        }
                    } else if (algoName === 'RSASSA-PKCS1-v1_5') {
                        if (key.type !== 'private' || key.algorithm.name !== 'RSASSA-PKCS1-v1_5') {
                            reject(new Error('Invalid key for RSA signing'));
                            return;
                        }

                        const hashName = key.algorithm.hash.name;
                        const result = __nativeRsaSign(hashName, key.__keyData, dataBytes);

                        if (result) {
                            resolve(result);
                        } else {
                            reject(new Error('RSA sign failed'));
                        }
                    } else {
                        reject(new Error('Unsupported algorithm: ' + algoName));
                    }
                } catch (e) {
                    reject(e);
                }
            });
        };

        // crypto.subtle.verify - HMAC, ECDSA, RSA
        crypto.subtle.verify = function(algorithm, key, signature, data) {
            return new Promise((resolve, reject) => {
                try {
                    const algoName = typeof algorithm === 'string' ? algorithm : algorithm.name;

                    let dataBytes, sigBytes;
                    if (data instanceof ArrayBuffer) {
                        dataBytes = new Uint8Array(data);
                    } else if (data instanceof Uint8Array) {
                        dataBytes = data;
                    } else {
                        reject(new Error('Data must be ArrayBuffer or Uint8Array'));
                        return;
                    }

                    if (signature instanceof ArrayBuffer) {
                        sigBytes = new Uint8Array(signature);
                    } else if (signature instanceof Uint8Array) {
                        sigBytes = signature;
                    } else {
                        reject(new Error('Signature must be ArrayBuffer or Uint8Array'));
                        return;
                    }

                    if (!key.__keyData) {
                        reject(new Error('Invalid key'));
                        return;
                    }

                    if (algoName === 'HMAC') {
                        const hashName = key.algorithm.hash.name;
                        const isValid = __nativeHmacVerify(hashName, key.__keyData, sigBytes, dataBytes);
                        resolve(isValid);
                    } else if (algoName === 'ECDSA') {
                        if (key.algorithm.name !== 'ECDSA') {
                            reject(new Error('Invalid key for ECDSA verification'));
                            return;
                        }

                        // For private keys, use the public key data
                        const publicKeyData = key.type === 'private' ? key.__publicKeyData : key.__keyData;
                        const isValid = __nativeEcdsaVerify(publicKeyData, sigBytes, dataBytes);
                        resolve(isValid);
                    } else if (algoName === 'RSASSA-PKCS1-v1_5') {
                        if (key.algorithm.name !== 'RSASSA-PKCS1-v1_5') {
                            reject(new Error('Invalid key for RSA verification'));
                            return;
                        }

                        const hashName = key.algorithm.hash.name;
                        const isValid = __nativeRsaVerify(hashName, key.__keyData, sigBytes, dataBytes);
                        resolve(isValid);
                    } else {
                        reject(new Error('Unsupported algorithm: ' + algoName));
                    }
                } catch (e) {
                    reject(e);
                }
            });
        };
    "#;

    context
        .evaluate_script(crypto_script, 1)
        .expect("Failed to setup crypto");
}
