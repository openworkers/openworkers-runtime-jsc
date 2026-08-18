use rusty_jsc::JSContext;
use rusty_jsc::JSValue;
use url::Url;

/// Serialize the WHATWG components JS reads back from the parser
fn parts_json(url: &Url) -> String {
    let port = url.port().map(|p| p.to_string()).unwrap_or_default();

    let host = match (url.host_str(), port.is_empty()) {
        (Some(host), false) => format!("{}:{}", host, port),
        (Some(host), true) => host.to_string(),
        (None, _) => String::new(),
    };

    let search = match url.query() {
        Some(query) if !query.is_empty() => format!("?{}", query),
        _ => String::new(),
    };

    let hash = match url.fragment() {
        Some(fragment) if !fragment.is_empty() => format!("#{}", fragment),
        _ => String::new(),
    };

    serde_json::json!({
        "href": url.as_str(),
        "protocol": format!("{}:", url.scheme()),
        "username": url.username(),
        "password": url.password().unwrap_or(""),
        "host": host,
        "hostname": url.host_str().unwrap_or(""),
        "port": port,
        "pathname": url.path(),
        "search": search,
        "hash": hash,
        "origin": url.origin().ascii_serialization(),
    })
    .to_string()
}

fn parse(input: &str, base: Option<&str>) -> Option<Url> {
    match base {
        Some(base) => Url::parse(base).ok()?.join(input).ok(),
        None => Url::parse(input).ok(),
    }
}

/// WHATWG setters ignore values they cannot apply, so every failure is silent
fn apply(url: &mut Url, part: &str, value: &str) {
    match part {
        "protocol" => {
            let _ = url.set_scheme(value.trim_end_matches(':'));
        }
        "username" => {
            let _ = url.set_username(value);
        }
        "password" => {
            let password = (!value.is_empty()).then_some(value);
            let _ = url.set_password(password);
        }
        "host" => {
            // An IPv6 literal is bracketed and carries colons of its own
            let host_end = value.rfind(']').map(|end| end + 1).unwrap_or(0);

            let (hostname, port) = match value[host_end..].find(':') {
                Some(colon) => (
                    &value[..host_end + colon],
                    Some(&value[host_end + colon + 1..]),
                ),
                None => (value, None),
            };

            if url.set_host(Some(hostname)).is_ok()
                && let Some(port) = port
                && let Ok(port) = port.parse::<u16>()
            {
                let _ = url.set_port(Some(port));
            }
        }
        "hostname" => {
            let _ = url.set_host(Some(value));
        }
        "port" => {
            if value.is_empty() {
                let _ = url.set_port(None);
            } else if let Ok(port) = value.parse::<u16>() {
                let _ = url.set_port(Some(port));
            }
        }
        "pathname" => url.set_path(value),
        "search" => {
            let query = value.trim_start_matches('?');
            url.set_query((!query.is_empty()).then_some(query));
        }
        "hash" => {
            let fragment = value.trim_start_matches('#');
            url.set_fragment((!fragment.is_empty()).then_some(fragment));
        }
        _ => {}
    }
}

fn string_arg(ctx: &JSContext, args: &[JSValue], index: usize) -> Option<String> {
    let value = args.get(index)?;

    if value.is_null(ctx) || value.is_undefined(ctx) {
        return None;
    }

    value.to_js_string(ctx).ok().map(|s| s.to_string())
}

/// Setup URL and URLSearchParams APIs
pub fn setup_url_api(context: &mut JSContext) {
    let parse_fn = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            let input = match string_arg(&ctx, args, 0) {
                Some(input) => input,
                None => return Ok(JSValue::null(&ctx)),
            };

            let base = string_arg(&ctx, args, 1);

            match parse(&input, base.as_deref()) {
                Some(url) => Ok(JSValue::string(&ctx, parts_json(&url).as_str())),
                None => Ok(JSValue::null(&ctx)),
            }
        }
    );

    let update_fn = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            let href = string_arg(&ctx, args, 0);
            let part = string_arg(&ctx, args, 1);
            let value = string_arg(&ctx, args, 2).unwrap_or_default();

            let mut url = match href.as_deref().and_then(|href| parse(href, None)) {
                Some(url) => url,
                None => return Ok(JSValue::null(&ctx)),
            };

            match part {
                Some(part) => apply(&mut url, &part, &value),
                None => return Ok(JSValue::null(&ctx)),
            }

            Ok(JSValue::string(&ctx, parts_json(&url).as_str()))
        }
    );

    let mut global = context.get_global_object();

    global
        .set_property(context, "__urlParse", parse_fn.into())
        .expect("global is writable");

    global
        .set_property(context, "__urlUpdate", update_fn.into())
        .expect("global is writable");

    context
        .evaluate_script(URL_JS, 1)
        .expect("Failed to setup URL");
}

const URL_JS: &str = r#"
(function() {
    const STATE = Symbol('urlState');
    const PARAMS = Symbol('urlSearchParams');
    const PAIRS = Symbol('pairs');
    const OWNER = Symbol('owner');

    const decodeValue = (input) => {
        const spaced = input.replace(/\+/g, ' ');

        try {
            return decodeURIComponent(spaced);
        } catch (e) {
            return spaced;
        }
    };

    // application/x-www-form-urlencoded keeps '*' but escapes what encodeURIComponent spares
    const encodeValue = (input) => encodeURIComponent(input)
        .replace(/[!'()~]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase())
        .replace(/%20/g, '+');

    const parseQuery = (input) => {
        const pairs = [];
        const query = input.charAt(0) === '?' ? input.slice(1) : input;

        for (const part of query.split('&')) {
            if (!part) {
                continue;
            }

            const eq = part.indexOf('=');
            const name = eq === -1 ? part : part.slice(0, eq);
            const value = eq === -1 ? '' : part.slice(eq + 1);

            pairs.push([decodeValue(name), decodeValue(value)]);
        }

        return pairs;
    };

    const update = (state, part, value) => {
        const updated = __urlUpdate(state.href, part, value);

        return updated === null ? state : JSON.parse(updated);
    };

    const syncOwner = (params) => {
        const owner = params[OWNER];

        if (owner) {
            owner[STATE] = update(owner[STATE], 'search', params.toString());
        }
    };

    class URLSearchParams {
        constructor(init) {
            this[PAIRS] = [];
            this[OWNER] = null;

            if (init === undefined || init === null) {
                return;
            }

            if (typeof init === 'string') {
                this[PAIRS] = parseQuery(init);
            } else if (init instanceof URLSearchParams) {
                this[PAIRS] = init[PAIRS].map(([name, value]) => [name, value]);
            } else if (typeof init[Symbol.iterator] === 'function') {
                for (const pair of init) {
                    const entry = Array.from(pair);

                    if (entry.length !== 2) {
                        throw new TypeError('URLSearchParams init must hold [name, value] pairs');
                    }

                    this[PAIRS].push([String(entry[0]), String(entry[1])]);
                }
            } else {
                for (const name of Object.keys(init)) {
                    this[PAIRS].push([name, String(init[name])]);
                }
            }
        }

        get size() {
            return this[PAIRS].length;
        }

        append(name, value) {
            this[PAIRS].push([String(name), String(value)]);
            syncOwner(this);
        }

        delete(name, value) {
            const key = String(name);
            const target = value === undefined ? undefined : String(value);

            this[PAIRS] = this[PAIRS].filter(([n, v]) => n !== key || (target !== undefined && v !== target));
            syncOwner(this);
        }

        get(name) {
            const key = String(name);
            const hit = this[PAIRS].find(([n]) => n === key);

            return hit === undefined ? null : hit[1];
        }

        getAll(name) {
            const key = String(name);

            return this[PAIRS].filter(([n]) => n === key).map(([, v]) => v);
        }

        has(name, value) {
            const key = String(name);
            const target = value === undefined ? undefined : String(value);

            return this[PAIRS].some(([n, v]) => n === key && (target === undefined || v === target));
        }

        set(name, value) {
            const key = String(name);
            const next = String(value);
            const index = this[PAIRS].findIndex(([n]) => n === key);

            if (index === -1) {
                this[PAIRS].push([key, next]);
            } else {
                this[PAIRS][index] = [key, next];
                this[PAIRS] = this[PAIRS].filter(([n], i) => n !== key || i === index);
            }

            syncOwner(this);
        }

        sort() {
            this[PAIRS].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
            syncOwner(this);
        }

        toString() {
            return this[PAIRS]
                .map(([name, value]) => encodeValue(name) + '=' + encodeValue(value))
                .join('&');
        }

        *entries() {
            for (const [name, value] of this[PAIRS]) {
                yield [name, value];
            }
        }

        *keys() {
            for (const [name] of this[PAIRS]) {
                yield name;
            }
        }

        *values() {
            for (const [, value] of this[PAIRS]) {
                yield value;
            }
        }

        forEach(callback, thisArg) {
            for (const [name, value] of this[PAIRS]) {
                callback.call(thisArg, value, name, this);
            }
        }

        [Symbol.iterator]() {
            return this.entries();
        }
    }

    const setPart = (url, part, value) => {
        url[STATE] = update(url[STATE], part, String(value));

        if (url[PARAMS]) {
            url[PARAMS][PAIRS] = parseQuery(url[STATE].search);
        }
    };

    class URL {
        constructor(input, base) {
            const href = String(input);
            const parsed = __urlParse(href, base === undefined || base === null ? null : String(base));

            if (parsed === null) {
                throw new TypeError(`Invalid URL: ${href}`);
            }

            this[STATE] = JSON.parse(parsed);
            this[PARAMS] = null;
        }

        get href() { return this[STATE].href; }
        set href(value) {
            const parsed = __urlParse(String(value), null);

            if (parsed === null) {
                throw new TypeError(`Invalid URL: ${value}`);
            }

            this[STATE] = JSON.parse(parsed);

            if (this[PARAMS]) {
                this[PARAMS][PAIRS] = parseQuery(this[STATE].search);
            }
        }

        get origin() { return this[STATE].origin; }

        get protocol() { return this[STATE].protocol; }
        set protocol(value) { setPart(this, 'protocol', value); }

        get username() { return this[STATE].username; }
        set username(value) { setPart(this, 'username', value); }

        get password() { return this[STATE].password; }
        set password(value) { setPart(this, 'password', value); }

        get host() { return this[STATE].host; }
        set host(value) { setPart(this, 'host', value); }

        get hostname() { return this[STATE].hostname; }
        set hostname(value) { setPart(this, 'hostname', value); }

        get port() { return this[STATE].port; }
        set port(value) { setPart(this, 'port', value); }

        get pathname() { return this[STATE].pathname; }
        set pathname(value) { setPart(this, 'pathname', value); }

        get search() { return this[STATE].search; }
        set search(value) { setPart(this, 'search', value); }

        get hash() { return this[STATE].hash; }
        set hash(value) { setPart(this, 'hash', value); }

        get searchParams() {
            if (!this[PARAMS]) {
                const params = new URLSearchParams(this[STATE].search);
                params[OWNER] = this;
                this[PARAMS] = params;
            }

            return this[PARAMS];
        }

        toString() { return this[STATE].href; }

        toJSON() { return this[STATE].href; }
    }

    globalThis.URL = URL;
    globalThis.URLSearchParams = URLSearchParams;
})();
"#;
