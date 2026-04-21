// media.codecs evasion
// Patches canPlayType and MediaSource.isTypeSupported to report common codecs
// as supported, even in headless environments that may lack them.
(function () {

    // Map of codecs to fake-support. These cover common detection checks.
    const codecSupport = {
        'video/mp4; codecs="avc1.42E01E"': "probably",
        "video/mp4": "maybe",
        'audio/mp4; codecs="mp4a.40.2"': "probably",
        'video/webm; codecs="vp8"': "probably",
        'video/webm; codecs="vp9"': "probably",
        'audio/webm; codecs="opus"': "probably",
        'audio/webm; codecs="vorbis"': "probably",
        'video/webm': "maybe",
        'audio/ogg; codecs="vorbis"': "probably",
    };

    // Patch HTMLMediaElement.prototype.canPlayType
    if (typeof HTMLMediaElement !== "undefined") {
        const origCanPlayType = HTMLMediaElement.prototype.canPlayType;
        const patchedCanPlayType = function canPlayType(type) {
            // Check our known list first
            if (type in codecSupport) {
                return codecSupport[type];
            }
            // Fall back to original
            return origCanPlayType.call(this, type);
        };
        utils.makeNativeToString(patchedCanPlayType, "canPlayType");
        HTMLMediaElement.prototype.canPlayType = patchedCanPlayType;
    }

    // Patch MediaSource.isTypeSupported
    if (typeof MediaSource !== "undefined") {
        const origIsTypeSupported = MediaSource.isTypeSupported;
        const patchedIsTypeSupported = function isTypeSupported(type) {
            // Return true for common codecs
            if (type in codecSupport) {
                return true;
            }
            return origIsTypeSupported.call(this, type);
        };
        utils.makeNativeToString(patchedIsTypeSupported, "isTypeSupported");
        MediaSource.isTypeSupported = patchedIsTypeSupported;
    }
})();
