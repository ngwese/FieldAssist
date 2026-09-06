// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

declare name "MonitorFoaFuma";
declare version "1.0";
declare license "MIT";

import("bformat.lib");

// FuMa first-order: maxN, channel order W X Y Z, converted to ACN/SN3D.
process = fuma_to_acn_sn3d : foa_stereo;
