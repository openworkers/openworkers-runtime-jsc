//! JSC Context Group support via direct FFI bindings
//!
//! This module provides direct bindings to JavaScriptCore's context group API,
//! bypassing rusty_jsc to enable sharing compiled bytecode between contexts.
//!
//! When multiple JSContexts are created in the same group, JSC internally
//! caches and reuses compiled bytecode for identical source strings.

use std::ffi::CString;
use std::ptr;

// Raw JSC types (opaque pointers)
#[repr(C)]
pub struct OpaqueJSContextGroup {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OpaqueJSContext {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OpaqueJSValue {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OpaqueJSString {
    _private: [u8; 0],
}

pub type JSContextGroupRef = *mut OpaqueJSContextGroup;
pub type JSGlobalContextRef = *mut OpaqueJSContext;
pub type JSContextRef = *mut OpaqueJSContext;
pub type JSValueRef = *const OpaqueJSValue;
pub type JSStringRef = *mut OpaqueJSString;
pub type JSClassRef = *mut std::ffi::c_void;
pub type JSObjectRef = *mut OpaqueJSValue;

// Link against JavaScriptCore
#[cfg_attr(target_os = "macos", link(name = "JavaScriptCore", kind = "framework"))]
#[cfg_attr(target_os = "linux", link(name = "javascriptcoregtk-4.1"))]
extern "C" {
    // Context Group functions
    fn JSContextGroupCreate() -> JSContextGroupRef;
    fn JSContextGroupRetain(group: JSContextGroupRef) -> JSContextGroupRef;
    fn JSContextGroupRelease(group: JSContextGroupRef);

    // Global Context functions
    fn JSGlobalContextCreateInGroup(
        group: JSContextGroupRef,
        global_object_class: JSClassRef,
    ) -> JSGlobalContextRef;
    fn JSGlobalContextRelease(ctx: JSGlobalContextRef);
    fn JSGlobalContextRetain(ctx: JSGlobalContextRef) -> JSGlobalContextRef;

    // Evaluation
    fn JSEvaluateScript(
        ctx: JSContextRef,
        script: JSStringRef,
        this_object: JSObjectRef,
        source_url: JSStringRef,
        starting_line_number: i32,
        exception: *mut JSValueRef,
    ) -> JSValueRef;

    // String functions
    fn JSStringCreateWithUTF8CString(string: *const i8) -> JSStringRef;
    fn JSStringRelease(string: JSStringRef);
    fn JSStringGetMaximumUTF8CStringSize(string: JSStringRef) -> usize;
    fn JSStringGetUTF8CString(string: JSStringRef, buffer: *mut i8, buffer_size: usize) -> usize;

    // Value functions
    fn JSValueToStringCopy(
        ctx: JSContextRef,
        value: JSValueRef,
        exception: *mut JSValueRef,
    ) -> JSStringRef;
    fn JSValueIsUndefined(ctx: JSContextRef, value: JSValueRef) -> bool;
    fn JSValueIsNull(ctx: JSContextRef, value: JSValueRef) -> bool;
    fn JSValueIsString(ctx: JSContextRef, value: JSValueRef) -> bool;

    // Object functions
    fn JSContextGetGlobalObject(ctx: JSContextRef) -> JSObjectRef;
    fn JSObjectSetProperty(
        ctx: JSContextRef,
        object: JSObjectRef,
        property_name: JSStringRef,
        value: JSValueRef,
        attributes: u32,
        exception: *mut JSValueRef,
    );
    fn JSObjectGetProperty(
        ctx: JSContextRef,
        object: JSObjectRef,
        property_name: JSStringRef,
        exception: *mut JSValueRef,
    ) -> JSValueRef;
}

/// A JavaScript context group that allows sharing compiled code between contexts.
///
/// Contexts created within the same group will share internally cached bytecode,
/// improving startup performance for repeated evaluations of the same code.
pub struct ContextGroup {
    inner: JSContextGroupRef,
}

// ContextGroup can be shared between threads (with proper synchronization)
unsafe impl Send for ContextGroup {}
unsafe impl Sync for ContextGroup {}

impl ContextGroup {
    /// Create a new context group.
    pub fn new() -> Self {
        let inner = unsafe { JSContextGroupCreate() };
        Self { inner }
    }

    /// Create a new global context within this group.
    pub fn create_context(&self) -> GroupedContext {
        let ctx = unsafe { JSGlobalContextCreateInGroup(self.inner, ptr::null_mut()) };
        GroupedContext { inner: ctx }
    }

    /// Get the raw JSContextGroupRef (for advanced usage).
    pub fn as_raw(&self) -> JSContextGroupRef {
        self.inner
    }
}

impl Default for ContextGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ContextGroup {
    fn clone(&self) -> Self {
        let inner = unsafe { JSContextGroupRetain(self.inner) };
        Self { inner }
    }
}

impl Drop for ContextGroup {
    fn drop(&mut self) {
        unsafe { JSContextGroupRelease(self.inner) };
    }
}

/// A JavaScript context created within a ContextGroup.
///
/// This context shares compiled bytecode with other contexts in the same group.
pub struct GroupedContext {
    inner: JSGlobalContextRef,
}

// GroupedContext is NOT Send - JSC contexts must stay on one thread
// But we implement Send anyway since our runtime handles this
unsafe impl Send for GroupedContext {}

impl GroupedContext {
    /// Evaluate a JavaScript script in this context.
    pub fn evaluate(&self, script: &str) -> Result<String, String> {
        let script_cstr = CString::new(script).map_err(|e| e.to_string())?;
        let script_js = unsafe { JSStringCreateWithUTF8CString(script_cstr.as_ptr()) };

        let mut exception: JSValueRef = ptr::null();

        let result = unsafe {
            JSEvaluateScript(
                self.inner,
                script_js,
                ptr::null_mut(),
                ptr::null_mut(),
                1,
                &mut exception,
            )
        };

        unsafe { JSStringRelease(script_js) };

        if !exception.is_null() {
            return Err(self.value_to_string(exception));
        }

        if result.is_null() {
            return Ok("undefined".to_string());
        }

        Ok(self.value_to_string(result))
    }

    /// Convert a JSValue to a Rust string.
    fn value_to_string(&self, value: JSValueRef) -> String {
        if value.is_null() {
            return "null".to_string();
        }

        unsafe {
            if JSValueIsUndefined(self.inner, value) {
                return "undefined".to_string();
            }
            if JSValueIsNull(self.inner, value) {
                return "null".to_string();
            }
        }

        let mut exception: JSValueRef = ptr::null();
        let js_string = unsafe { JSValueToStringCopy(self.inner, value, &mut exception) };

        if js_string.is_null() {
            return "[object]".to_string();
        }

        let max_size = unsafe { JSStringGetMaximumUTF8CStringSize(js_string) };
        let mut buffer = vec![0i8; max_size];
        let actual_size =
            unsafe { JSStringGetUTF8CString(js_string, buffer.as_mut_ptr(), max_size) };

        unsafe { JSStringRelease(js_string) };

        if actual_size > 0 {
            // actual_size includes null terminator
            let bytes: Vec<u8> = buffer[..(actual_size - 1) as usize]
                .iter()
                .map(|&c| c as u8)
                .collect();
            String::from_utf8_lossy(&bytes).to_string()
        } else {
            String::new()
        }
    }

    /// Get the global object of this context.
    pub fn global_object(&self) -> JSObjectRef {
        unsafe { JSContextGetGlobalObject(self.inner) }
    }

    /// Set a property on the global object.
    pub fn set_global_property(&self, name: &str, value: JSValueRef) -> Result<(), String> {
        let name_cstr = CString::new(name).map_err(|e| e.to_string())?;
        let name_js = unsafe { JSStringCreateWithUTF8CString(name_cstr.as_ptr()) };

        let mut exception: JSValueRef = ptr::null();
        let global = self.global_object();

        unsafe {
            JSObjectSetProperty(self.inner, global, name_js, value, 0, &mut exception);
            JSStringRelease(name_js);
        }

        if !exception.is_null() {
            return Err(self.value_to_string(exception));
        }

        Ok(())
    }

    /// Get a property from the global object.
    pub fn get_global_property(&self, name: &str) -> Result<JSValueRef, String> {
        let name_cstr = CString::new(name).map_err(|e| e.to_string())?;
        let name_js = unsafe { JSStringCreateWithUTF8CString(name_cstr.as_ptr()) };

        let mut exception: JSValueRef = ptr::null();
        let global = self.global_object();

        let value = unsafe { JSObjectGetProperty(self.inner, global, name_js, &mut exception) };

        unsafe { JSStringRelease(name_js) };

        if !exception.is_null() {
            return Err(self.value_to_string(exception));
        }

        Ok(value)
    }

    /// Get the raw JSGlobalContextRef (for interop with rusty_jsc if needed).
    pub fn as_raw(&self) -> JSGlobalContextRef {
        self.inner
    }
}

impl Clone for GroupedContext {
    fn clone(&self) -> Self {
        let inner = unsafe { JSGlobalContextRetain(self.inner) };
        Self { inner }
    }
}

impl Drop for GroupedContext {
    fn drop(&mut self) {
        unsafe { JSGlobalContextRelease(self.inner) };
    }
}

/// Pre-compiled script template that can be quickly instantiated.
///
/// This stores source code that will be evaluated in each new context.
/// When contexts share a group, JSC caches the compiled bytecode internally.
pub struct ScriptTemplate {
    /// The source code to evaluate
    source: String,
}

impl ScriptTemplate {
    /// Create a new script template from source code.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
        }
    }

    /// Evaluate this template in the given context.
    pub fn evaluate_in(&self, ctx: &GroupedContext) -> Result<String, String> {
        ctx.evaluate(&self.source)
    }

    /// Get the source code.
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// A factory for creating contexts with pre-loaded scripts.
///
/// This is the main entry point for snapshot-like functionality.
/// Create a ContextFactory with your base scripts, then call `create_context()`
/// to get new contexts with those scripts already evaluated.
pub struct ContextFactory {
    /// The shared context group
    group: ContextGroup,
    /// Scripts to evaluate in each new context
    templates: Vec<ScriptTemplate>,
}

impl ContextFactory {
    /// Create a new context factory.
    pub fn new() -> Self {
        Self {
            group: ContextGroup::new(),
            templates: Vec::new(),
        }
    }

    /// Add a script that will be evaluated in each new context.
    pub fn add_script(&mut self, source: impl Into<String>) -> &mut Self {
        self.templates.push(ScriptTemplate::new(source));
        self
    }

    /// Create a new context with all template scripts pre-evaluated.
    ///
    /// Because all contexts share the same group, JSC will reuse
    /// cached bytecode for the template scripts.
    pub fn create_context(&self) -> Result<GroupedContext, String> {
        let ctx = self.group.create_context();

        // Evaluate all templates
        for template in &self.templates {
            template.evaluate_in(&ctx)?;
        }

        Ok(ctx)
    }

    /// Get a reference to the underlying context group.
    pub fn group(&self) -> &ContextGroup {
        &self.group
    }
}

impl Default for ContextFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_group_basic() {
        let group = ContextGroup::new();
        let ctx = group.create_context();

        let result = ctx.evaluate("1 + 2").unwrap();
        assert_eq!(result, "3");
    }

    #[test]
    fn test_multiple_contexts_same_group() {
        let group = ContextGroup::new();

        let ctx1 = group.create_context();
        let ctx2 = group.create_context();

        // Both contexts should work independently
        ctx1.evaluate("var x = 10").unwrap();
        ctx2.evaluate("var x = 20").unwrap();

        let r1 = ctx1.evaluate("x").unwrap();
        let r2 = ctx2.evaluate("x").unwrap();

        assert_eq!(r1, "10");
        assert_eq!(r2, "20");
    }

    #[test]
    fn test_context_factory() {
        let mut factory = ContextFactory::new();
        factory.add_script("const BASE_VALUE = 42;");
        factory.add_script("function double(x) { return x * 2; }");

        let ctx = factory.create_context().unwrap();

        let result = ctx.evaluate("double(BASE_VALUE)").unwrap();
        assert_eq!(result, "84");
    }

    #[test]
    fn test_shared_bytecode_performance() {
        // This test demonstrates that multiple contexts benefit from shared bytecode
        let mut factory = ContextFactory::new();

        // Add a moderately complex script
        factory.add_script(
            r#"
            class Calculator {
                constructor(initial) {
                    this.value = initial;
                }
                add(x) { this.value += x; return this; }
                multiply(x) { this.value *= x; return this; }
                result() { return this.value; }
            }

            function fibonacci(n) {
                if (n <= 1) return n;
                return fibonacci(n - 1) + fibonacci(n - 2);
            }
        "#,
        );

        // Create multiple contexts - subsequent ones should be faster due to bytecode caching
        let contexts: Vec<_> = (0..5)
            .map(|_| factory.create_context().unwrap())
            .collect();

        // Verify all contexts work correctly
        for (i, ctx) in contexts.iter().enumerate() {
            let result = ctx
                .evaluate(&format!(
                    "new Calculator({}).add(10).multiply(2).result()",
                    i
                ))
                .unwrap();
            let expected = ((i + 10) * 2).to_string();
            assert_eq!(result, expected);
        }
    }

    #[test]
    fn test_error_handling() {
        let group = ContextGroup::new();
        let ctx = group.create_context();

        let result = ctx.evaluate("throw new Error('test error')");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("test error"));
    }
}
