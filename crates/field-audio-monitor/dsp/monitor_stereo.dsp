// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

declare name "MonitorStereo";
declare version "1.0";
declare license "MIT";

import("stdfaust.lib");
import("headphone_crossfeed.lib");

gainDb = hslider("Monitor/Gain [unit:dB][style:knob]",
                 0, -90, 12, 0.1) : ba.db2linear : si.smoo;

width = hslider("Stereo/Width [unit:%]", 100, 0, 200, 1) / 100 : si.smoo;

encodeMS(l, r) = ((l + r) * 0.5, (l - r) * 0.5);
decodeMS(m, s) = (m + s, m - s);

stereo = encodeMS : (*(gainDb), *(width)) : decodeMS;

process = stereo : headphone_crossfeed;
