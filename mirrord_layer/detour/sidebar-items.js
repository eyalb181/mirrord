window.SIDEBAR_ITEMS = {"constant":[["DETOUR_BYPASS","Holds the thread-local state for bypassing the layer’s detour functions."]],"enum":[["Bypass","Soft-errors that can be recovered from by calling the raw FFI function."],["Detour","`ControlFlow`-like enum to be used by hooks."]],"fn":[["detour_bypass_off","Sets [`DETOUR_BYPASS`] to `false`."],["detour_bypass_on","Sets [`DETOUR_BYPASS`] to `true`, bypassing the layer’s detours."]],"struct":[["DetourGuard","Handler for the layer’s [`DETOUR_BYPASS`]."],["HookFn","Wrapper around `OnceLock`, mainly used for the [`Deref`] implementation to simplify calls to the original functions as `FN_ORIGINAL()`, instead of `FN_ORIGINAL.get().unwrap()`."]],"trait":[["OnceLockExt","Extends [`OnceLock`] with a helper function to initialize it with a [`Detour`]."],["OptionExt","Extends `Option<T>` with the `Option::bypass` function."]]};