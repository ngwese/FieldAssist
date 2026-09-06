declare name "MonitorFoa";
declare version "1.0";
declare license "MIT";

import("stdfaust.lib");

// Ambix first-order: ACN + SN3D, channel order W Y Z X.
gainDb = hslider("Monitor/Gain [unit:dB][style:knob]",
                 0, -90, 12, 0.1) : ba.db2linear : si.smoo;

yawDeg = hslider("Ambisonics/Yaw [unit:deg]", 0, -180, 180, 0.1) : si.smoo;

// Horizontal stereo speakers at ±30°. Z is unused.
decode(w, y, z, x) = l * gainDb, r * gainDb
with {
    a = yawDeg * ma.PI / 180.0;
    c = cos(a);
    s = sin(a);
    xp = x * c + y * s;
    yp = -x * s + y * c;
    w0 = w * 0.7071067811865476;
    l = w0 + xp * 0.8660254037844386 + yp * 0.5;
    r = w0 + xp * 0.8660254037844386 - yp * 0.5;
};

process = decode;
