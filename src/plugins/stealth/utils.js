// Stealth utility functions — must be injected FIRST before all other evasion scripts.
// Based on puppeteer-extra-plugin-stealth's _utils approach.
(function () {
    if (window._pup_utils) return; // already injected

    // Cache native references early, before any site code can tamper with them.
    const _Reflect = Reflect;
    const _Object = Object;
    const _Function = Function;
    const _Error = Error;

    /**
     * Make a function's toString() return "[native code]" as if it were a built-in.
     */
    function makeNativeToString(fn, name) {
        const nativeStr = `function ${name || fn.name || ""}() { [native code] }`;
        // Replace toString on the function itself
        const toStringHandler = {
            apply: function (target, thisArg, args) {
                // If called on our patched function, return native string
                if (thisArg === fn) {
                    return nativeStr;
                }
                // Otherwise call original toString
                return _Reflect.apply(target, thisArg, args);
            },
        };
        const originalToString =
            _Function.prototype.toString;
        const patchedToString = new Proxy(originalToString, toStringHandler);

        // Override toString for this specific function
        _Object.defineProperty(fn, "toString", {
            value: patchedToString,
            writable: true,
            configurable: true,
        });
        // Also handle Function.prototype.toString.call(fn)
        return fn;
    }

    /**
     * Wrap a Proxy handler so that any errors thrown don't leak "Proxy" in stack traces.
     */
    function stripProxyFromErrors(handler) {
        const newHandler = {};
        for (const trapName of _Object.getOwnPropertyNames(handler)) {
            newHandler[trapName] = function () {
                try {
                    return handler[trapName].apply(this, arguments);
                } catch (err) {
                    if (err && err.stack) {
                        err.stack = err.stack.replace(
                            /at .*Proxy\./g,
                            "at Object."
                        );
                    }
                    throw err;
                }
            };
        }
        return newHandler;
    }

    /**
     * Override a property getter with native-looking toString.
     */
    function overridePropertyGetter(obj, propName, getter) {
        const descriptor =
            _Object.getOwnPropertyDescriptor(obj, propName) || {};
        const newGetter = makeNativeToString(getter, `get ${propName}`);
        _Object.defineProperty(obj, propName, {
            ...descriptor,
            get: newGetter,
            set: undefined,
            configurable: true,
        });
    }

    /**
     * Override a function property to look native.
     */
    function overrideFunction(obj, propName, fn) {
        const patched = makeNativeToString(fn, propName);
        _Object.defineProperty(obj, propName, {
            value: patched,
            writable: true,
            configurable: true,
        });
    }

    /**
     * Creates a mock class instance that looks real (prototype chain, toString, etc.).
     */
    function mockClass(mockObj, className) {
        // Create a proper constructor function
        const handler = {
            construct(target, args) {
                return _Object.create(mockObj);
            },
        };
        const MockClass = new Proxy(function () { }, handler);
        _Object.defineProperty(MockClass, "name", { value: className });
        _Object.defineProperty(MockClass.prototype, Symbol.toStringTag, {
            value: className,
            configurable: true,
        });
        makeNativeToString(MockClass, className);
        return MockClass;
    }

    window._pup_utils = {
        makeNativeToString,
        stripProxyFromErrors,
        overridePropertyGetter,
        overrideFunction,
        mockClass,
        cache: { Reflect: _Reflect, Object: _Object, Function: _Function, Error: _Error },
    };
})();
