use ring::{digest, hmac, rand};
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

        // crypto.subtle.importKey for HMAC
        crypto.subtle.importKey = function(format, keyData, algorithm, extractable, keyUsages) {
            return new Promise((resolve, reject) => {
                try {
                    if (format !== 'raw') {
                        reject(new Error('Only "raw" format is supported'));
                        return;
                    }

                    const algoName = typeof algorithm === 'string' ? algorithm : algorithm.name;
                    const hashName = typeof algorithm === 'object' && algorithm.hash
                        ? (typeof algorithm.hash === 'string' ? algorithm.hash : algorithm.hash.name)
                        : 'SHA-256';

                    if (algoName !== 'HMAC') {
                        reject(new Error('Only HMAC algorithm is supported'));
                        return;
                    }

                    let keyBytes;
                    if (keyData instanceof ArrayBuffer) {
                        keyBytes = new Uint8Array(keyData);
                    } else if (keyData instanceof Uint8Array) {
                        keyBytes = keyData;
                    } else {
                        reject(new Error('Key data must be ArrayBuffer or Uint8Array'));
                        return;
                    }

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
                } catch (e) {
                    reject(e);
                }
            });
        };

        // crypto.subtle.sign for HMAC
        crypto.subtle.sign = function(algorithm, key, data) {
            return new Promise((resolve, reject) => {
                try {
                    const algoName = typeof algorithm === 'string' ? algorithm : algorithm.name;

                    if (algoName !== 'HMAC') {
                        reject(new Error('Only HMAC algorithm is supported'));
                        return;
                    }

                    if (!key.__keyData) {
                        reject(new Error('Invalid key'));
                        return;
                    }

                    let dataBytes;
                    if (data instanceof ArrayBuffer) {
                        dataBytes = new Uint8Array(data);
                    } else if (data instanceof Uint8Array) {
                        dataBytes = data;
                    } else {
                        reject(new Error('Data must be ArrayBuffer or Uint8Array'));
                        return;
                    }

                    const hashName = key.algorithm.hash.name;
                    const result = __nativeHmacSign(hashName, key.__keyData, dataBytes);

                    if (result) {
                        resolve(result);
                    } else {
                        reject(new Error('Sign failed'));
                    }
                } catch (e) {
                    reject(e);
                }
            });
        };

        // crypto.subtle.verify for HMAC
        crypto.subtle.verify = function(algorithm, key, signature, data) {
            return new Promise((resolve, reject) => {
                try {
                    const algoName = typeof algorithm === 'string' ? algorithm : algorithm.name;

                    if (algoName !== 'HMAC') {
                        reject(new Error('Only HMAC algorithm is supported'));
                        return;
                    }

                    if (!key.__keyData) {
                        reject(new Error('Invalid key'));
                        return;
                    }

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

                    const hashName = key.algorithm.hash.name;
                    const isValid = __nativeHmacVerify(hashName, key.__keyData, sigBytes, dataBytes);

                    resolve(isValid);
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
