// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

declare name "MonitorStereo";
declare version "1.0";
declare license "MIT";

import("stdfaust.lib");
import("meters.lib");
import("headphone_crossfeed.lib");

process = meterInputL, meterInputR
        : headphone_crossfeed
        : *(outputGain), *(outputGain)
        : meterOutputL, meterOutputR;
