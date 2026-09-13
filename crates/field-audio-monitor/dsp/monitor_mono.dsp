// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

declare name "MonitorMono";
declare version "1.0";
declare license "MIT";

import("stdfaust.lib");
import("meters.lib");

process = meterInput : *(outputGain) <: meterOutputL, meterOutputR;
