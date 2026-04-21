// navigator.permissions evasion
// Overrides Permissions.query() to return 'prompt' for the 'notifications'
// permission. Headless Chrome returns 'denied' which is a detection signal.
(function () {

    if (typeof Permissions === "undefined") return;

    const originalQuery = Permissions.prototype.query;

    const patchedQuery = function query(parameters) {
        // Notifications permission in headless returns "denied" → fake it as "prompt"
        if (parameters && parameters.name === "notifications") {
            return Promise.resolve({
                state: "prompt",
                onchange: null,
                addEventListener: function () { },
                removeEventListener: function () { },
                dispatchEvent: function () {
                    return true;
                },
            });
        }
        return originalQuery.call(this, parameters);
    };

    utils.makeNativeToString(patchedQuery, "query");
    Permissions.prototype.query = patchedQuery;
})();
