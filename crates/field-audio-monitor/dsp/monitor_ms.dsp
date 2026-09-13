// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

declare name "MonitorMs";
declare version "1.0";
declare license "MIT";

import("stdfaust.lib");
import("meters.lib");
import("headphone_crossfeed.lib");

midTrimDb = hslider("M-S/Mid trim [unit:dB]", 0, -24, 24, 0.1)
            : ba.db2linear : si.smoo;

sideTrimDb = hslider("M-S/Side trim [unit:dB]", 0, -24, 24, 0.1)
             : ba.db2linear : si.smoo;

sideFlip = 1 - 2 * checkbox("M-S/Invert side");

decodeMS(m, s) = m + s, m - s;

// Unscaled decode: L = M+S, R = M-S (up to +6 dB for correlated peaks).
decoded = *(midTrimDb), *(sideTrimDb * sideFlip) : decodeMS;

process = meterInputM, meterInputS
        : decoded
        : headphone_crossfeed
        : *(outputGain), *(outputGain)
        : meterOutputL, meterOutputR;
