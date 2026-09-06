// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

declare name "MonitorMono";
declare version "1.0";
declare license "MIT";

import("stdfaust.lib");

gainDb = hslider("Monitor/Gain [unit:dB][style:knob]",
                 0, -90, 12, 0.1) : ba.db2linear : si.smoo;

meter(x) = attach(x, ba.linear2db(max(1e-12, abs(x))) :
              hbargraph("Meters/Input peak [unit:dB]", -90, 6));

process = _ : meter : *(gainDb) <: _, _;
