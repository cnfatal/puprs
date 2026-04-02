// navigator.plugins evasion
// Creates a realistic PluginArray with common Chrome plugins.
// This is one of the most sophisticated evasions since headless Chrome
// has an empty plugin list.
(function () {
    const utils = window._pup_utils;
    if (!utils) return;
    const { makeNativeToString, cache } = utils;
    const _Object = cache.Object;

    // Define the plugins that a normal Chrome on macOS/Windows would have
    const pluginData = [
        {
            name: "PDF Viewer",
            description: "Portable Document Format",
            filename: "internal-pdf-viewer",
            mimeTypes: [
                { type: "application/pdf", suffixes: "pdf", description: "Portable Document Format" },
            ],
        },
        {
            name: "Chrome PDF Viewer",
            description: "Portable Document Format",
            filename: "internal-pdf-viewer",
            mimeTypes: [
                { type: "application/pdf", suffixes: "pdf", description: "Portable Document Format" },
            ],
        },
        {
            name: "Chromium PDF Viewer",
            description: "Portable Document Format",
            filename: "internal-pdf-viewer",
            mimeTypes: [
                { type: "application/pdf", suffixes: "pdf", description: "Portable Document Format" },
            ],
        },
        {
            name: "Microsoft Edge PDF Viewer",
            description: "Portable Document Format",
            filename: "internal-pdf-viewer",
            mimeTypes: [
                { type: "application/pdf", suffixes: "pdf", description: "Portable Document Format" },
            ],
        },
        {
            name: "WebKit built-in PDF",
            description: "Portable Document Format",
            filename: "internal-pdf-viewer",
            mimeTypes: [
                { type: "application/pdf", suffixes: "pdf", description: "Portable Document Format" },
            ],
        },
    ];

    // Create MimeType-like objects
    function makeMimeType(mt, plugin) {
        const obj = _Object.create(MimeType.prototype);
        _Object.defineProperties(obj, {
            type: { get: makeNativeToString(() => mt.type, "get type"), enumerable: true },
            suffixes: { get: makeNativeToString(() => mt.suffixes, "get suffixes"), enumerable: true },
            description: {
                get: makeNativeToString(() => mt.description, "get description"),
                enumerable: true,
            },
            enabledPlugin: {
                get: makeNativeToString(() => plugin, "get enabledPlugin"),
                enumerable: true,
            },
        });
        return obj;
    }

    // Create Plugin-like objects
    function makePlugin(pd) {
        const obj = _Object.create(Plugin.prototype);
        const mimeTypes = pd.mimeTypes.map((mt) => makeMimeType(mt, obj));

        _Object.defineProperties(obj, {
            name: { get: makeNativeToString(() => pd.name, "get name"), enumerable: true },
            description: {
                get: makeNativeToString(() => pd.description, "get description"),
                enumerable: true,
            },
            filename: { get: makeNativeToString(() => pd.filename, "get filename"), enumerable: true },
            length: { get: makeNativeToString(() => mimeTypes.length, "get length"), enumerable: true },
        });

        // Index access
        mimeTypes.forEach((mt, i) => {
            _Object.defineProperty(obj, i, { value: mt, writable: false, enumerable: false });
        });

        // namedItem / item
        obj.item = makeNativeToString(function item(index) {
            return mimeTypes[index] || null;
        }, "item");

        obj.namedItem = makeNativeToString(function namedItem(name) {
            return mimeTypes.find((mt) => mt.type === name) || null;
        }, "namedItem");

        obj[Symbol.iterator] = makeNativeToString(function* () {
            for (const mt of mimeTypes) yield mt;
        }, "[Symbol.iterator]");

        return obj;
    }

    const plugins = pluginData.map(makePlugin);
    const allMimeTypes = plugins.flatMap((p) => {
        const mts = [];
        for (let i = 0; i < p.length; i++) mts.push(p[i]);
        return mts;
    });

    // Create PluginArray-like object
    const pluginArray = _Object.create(PluginArray.prototype);
    plugins.forEach((p, i) => {
        _Object.defineProperty(pluginArray, i, { value: p, writable: false, enumerable: true });
    });

    _Object.defineProperties(pluginArray, {
        length: {
            get: makeNativeToString(() => plugins.length, "get length"),
            enumerable: true,
        },
    });

    pluginArray.item = makeNativeToString(function item(index) {
        return plugins[index] || null;
    }, "item");

    pluginArray.namedItem = makeNativeToString(function namedItem(name) {
        return plugins.find((p) => p.name === name) || null;
    }, "namedItem");

    pluginArray.refresh = makeNativeToString(function refresh() { }, "refresh");

    pluginArray[Symbol.iterator] = makeNativeToString(function* () {
        for (const p of plugins) yield p;
    }, "[Symbol.iterator]");

    // Create MimeTypeArray-like object
    const mimeTypeArray = _Object.create(MimeTypeArray.prototype);
    allMimeTypes.forEach((mt, i) => {
        _Object.defineProperty(mimeTypeArray, i, { value: mt, writable: false, enumerable: true });
    });

    _Object.defineProperties(mimeTypeArray, {
        length: {
            get: makeNativeToString(() => allMimeTypes.length, "get length"),
            enumerable: true,
        },
    });

    mimeTypeArray.item = makeNativeToString(function item(index) {
        return allMimeTypes[index] || null;
    }, "item");

    mimeTypeArray.namedItem = makeNativeToString(function namedItem(name) {
        return allMimeTypes.find((mt) => mt.type === name) || null;
    }, "namedItem");

    mimeTypeArray[Symbol.iterator] = makeNativeToString(function* () {
        for (const mt of allMimeTypes) yield mt;
    }, "[Symbol.iterator]");

    // Override navigator.plugins and navigator.mimeTypes
    utils.overridePropertyGetter(
        Object.getPrototypeOf(navigator),
        "plugins",
        function () {
            return pluginArray;
        }
    );

    utils.overridePropertyGetter(
        Object.getPrototypeOf(navigator),
        "mimeTypes",
        function () {
            return mimeTypeArray;
        }
    );
})();
