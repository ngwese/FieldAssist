/* ------------------------------------------------------------
license: "MIT"
name: "MonitorStereo"
version: "1.0"
Code generated with Faust 2.85.9 (https://faust.grame.fr)
Compilation options: -lang rust -fpga-mem-th 4 -ct 1 -cn MonitorStereo -es 1 -mcd 16 -mdd 1024 -mdy 33 -single -ftz 0
------------------------------------------------------------ */

#[repr(C)]
pub struct MonitorStereo {
	fCheckbox0: FaustFloat,
	fSampleRate: i32,
	fConst0: F32,
	fConst1: F32,
	fConst2: F32,
	fHslider0: FaustFloat,
	fRec1: [F32;2],
	fConst3: F32,
	fHslider1: FaustFloat,
	fConst4: F32,
	fRec2: [F32;2],
	fHslider2: FaustFloat,
	fRec3: [F32;2],
	fVec0: [F32;2],
	fRec0: [F32;2],
	fHslider3: FaustFloat,
	fRec4: [F32;2],
	fVec1: [F32;2],
	fRec5: [F32;2],
}


pub const FAUST_INPUTS: usize = 2;
pub const FAUST_OUTPUTS: usize = 2;
pub const FAUST_ACTIVES: usize = 5;
pub const FAUST_PASSIVES: usize = 0;

impl MonitorStereo {
		
	pub fn new() -> MonitorStereo { 
		MonitorStereo {
			fCheckbox0: 0.0,
			fSampleRate: 0,
			fConst0: 0.0,
			fConst1: 0.0,
			fConst2: 0.0,
			fHslider0: 0.0,
			fRec1: [0.0;2],
			fConst3: 0.0,
			fHslider1: 0.0,
			fConst4: 0.0,
			fRec2: [0.0;2],
			fHslider2: 0.0,
			fRec3: [0.0;2],
			fVec0: [0.0;2],
			fRec0: [0.0;2],
			fHslider3: 0.0,
			fRec4: [0.0;2],
			fVec1: [0.0;2],
			fRec5: [0.0;2],
		}
	}
	pub fn metadata(&self, m: &mut dyn Meta) { 
		m.declare("basics.lib/bypass2:author", r"Julius Smith");
		m.declare("basics.lib/name", r"Faust Basic Element Library");
		m.declare("basics.lib/version", r"1.22.0");
		m.declare("compile_options", r"-lang rust -fpga-mem-th 4 -ct 1 -cn MonitorStereo -es 1 -mcd 16 -mdd 1024 -mdy 33 -single -ftz 0");
		m.declare("filename", r"monitor_stereo.dsp");
		m.declare("filters.lib/lowpass0_highpass1", r"MIT-style STK-4.3 license");
		m.declare("filters.lib/lowpass0_highpass1:author", r"Julius O. Smith III");
		m.declare("filters.lib/lowpass:author", r"Julius O. Smith III");
		m.declare("filters.lib/lowpass:copyright", r"Copyright (C) 2003-2019 by Julius O. Smith III <jos@ccrma.stanford.edu>");
		m.declare("filters.lib/lowpass:license", r"MIT-style STK-4.3 license");
		m.declare("filters.lib/name", r"Faust Filters Library");
		m.declare("filters.lib/tf1:author", r"Julius O. Smith III");
		m.declare("filters.lib/tf1:copyright", r"Copyright (C) 2003-2019 by Julius O. Smith III <jos@ccrma.stanford.edu>");
		m.declare("filters.lib/tf1:license", r"MIT-style STK-4.3 license");
		m.declare("filters.lib/tf1s:author", r"Julius O. Smith III");
		m.declare("filters.lib/tf1s:copyright", r"Copyright (C) 2003-2019 by Julius O. Smith III <jos@ccrma.stanford.edu>");
		m.declare("filters.lib/tf1s:license", r"MIT-style STK-4.3 license");
		m.declare("filters.lib/version", r"1.7.1");
		m.declare("license", r"MIT");
		m.declare("maths.lib/author", r"GRAME");
		m.declare("maths.lib/copyright", r"GRAME");
		m.declare("maths.lib/license", r"LGPL with exception");
		m.declare("maths.lib/name", r"Faust Math Library");
		m.declare("maths.lib/version", r"2.9.0");
		m.declare("name", r"MonitorStereo");
		m.declare("platform.lib/name", r"Generic Platform Library");
		m.declare("platform.lib/version", r"1.3.0");
		m.declare("routes.lib/name", r"Faust Signal Routing Library");
		m.declare("routes.lib/version", r"1.3.0");
		m.declare("signals.lib/name", r"Faust Routing Library");
		m.declare("signals.lib/version", r"1.6.0");
		m.declare("version", r"1.0");
	}

	pub fn get_sample_rate(&self) -> i32 { self.fSampleRate as i32}
	
	pub fn class_init(sample_rate: i32) {
		// Obtaining locks on 0 static var(s)
	}
	pub fn instance_reset_params(&mut self) {
		self.fCheckbox0 = (0.0) as FaustFloat;
		self.fHslider0 = (7e+02) as FaustFloat;
		self.fHslider1 = (1e+02) as FaustFloat;
		self.fHslider2 = (0.0) as FaustFloat;
		self.fHslider3 = (0.35) as FaustFloat;
	}
	pub fn instance_clear(&mut self) {
		for l0 in 0..2 {
			self.fRec1[l0 as usize] = 0.0;
		}
		for l1 in 0..2 {
			self.fRec2[l1 as usize] = 0.0;
		}
		for l2 in 0..2 {
			self.fRec3[l2 as usize] = 0.0;
		}
		for l3 in 0..2 {
			self.fVec0[l3 as usize] = 0.0;
		}
		for l4 in 0..2 {
			self.fRec0[l4 as usize] = 0.0;
		}
		for l5 in 0..2 {
			self.fRec4[l5 as usize] = 0.0;
		}
		for l6 in 0..2 {
			self.fVec1[l6 as usize] = 0.0;
		}
		for l7 in 0..2 {
			self.fRec5[l7 as usize] = 0.0;
		}
	}
	pub fn instance_constants(&mut self, sample_rate: i32) {
		// Obtaining locks on 0 static var(s)
		self.fSampleRate = sample_rate;
		self.fConst0 = F32::min(1.92e+05, F32::max(1.0, (self.fSampleRate) as F32));
		self.fConst1 = 44.1 / self.fConst0;
		self.fConst2 = 1.0 - self.fConst1;
		self.fConst3 = 3.1415927 / self.fConst0;
		self.fConst4 = 0.441 / self.fConst0;
	}
	pub fn instance_init(&mut self, sample_rate: i32) {
		self.instance_constants(sample_rate);
		self.instance_reset_params();
		self.instance_clear();
	}
	pub fn init(&mut self, sample_rate: i32) {
		MonitorStereo::class_init(sample_rate);
		self.instance_init(sample_rate);
	}
	
	pub fn build_user_interface(&self, ui_interface: &mut dyn UI<FaustFloat>) {
		Self::build_user_interface_static(ui_interface);
	}
	
	pub fn build_user_interface_static(ui_interface: &mut dyn UI<FaustFloat>) {
		ui_interface.open_vertical_box("MonitorStereo");
		ui_interface.add_horizontal_slider("Headphones/Amount", ParamIndex(0), 0.35, 0.0, 1.0, 0.01);
		ui_interface.add_check_button("Headphones/Crossfeed", ParamIndex(1));
		ui_interface.declare(Some(ParamIndex(2)), "scale", "log");
		ui_interface.declare(Some(ParamIndex(2)), "unit", "Hz");
		ui_interface.add_horizontal_slider("Headphones/Crossover", ParamIndex(2), 7e+02, 3e+02, 2e+03, 1.0);
		ui_interface.declare(Some(ParamIndex(3)), "style", "knob");
		ui_interface.declare(Some(ParamIndex(3)), "unit", "dB");
		ui_interface.add_horizontal_slider("Monitor/Gain", ParamIndex(3), 0.0, -9e+01, 12.0, 0.1);
		ui_interface.declare(Some(ParamIndex(4)), "unit", "%");
		ui_interface.add_horizontal_slider("Stereo/Width", ParamIndex(4), 1e+02, 0.0, 2e+02, 1.0);
		ui_interface.close_box();
	}
	
	pub fn get_param(&self, param: ParamIndex) -> Option<FaustFloat> {
		match param.0 {
			1 => Some(self.fCheckbox0),
			2 => Some(self.fHslider0),
			4 => Some(self.fHslider1),
			3 => Some(self.fHslider2),
			0 => Some(self.fHslider3),
			_ => None,
		}
	}
	
	pub fn set_param(&mut self, param: ParamIndex, value: FaustFloat) {
		match param.0 {
			1 => { self.fCheckbox0 = value }
			2 => { self.fHslider0 = value }
			4 => { self.fHslider1 = value }
			3 => { self.fHslider2 = value }
			0 => { self.fHslider3 = value }
			_ => {}
		}
	}
	
	pub fn compute(
		&mut self,
		count: usize,
		inputs: &[impl AsRef<[FaustFloat]>],
		outputs: &mut[impl AsMut<[FaustFloat]>],
	) {
		
		// Obtaining locks on 0 static var(s)
		let [inputs0, inputs1, .. ] = inputs.as_ref() else { panic!("wrong number of input buffers"); };
		let inputs0 = inputs0.as_ref()[..count].iter();
		let inputs1 = inputs1.as_ref()[..count].iter();
		let [outputs0, outputs1, .. ] = outputs.as_mut() else { panic!("wrong number of output buffers"); };
		let outputs0 = outputs0.as_mut()[..count].iter_mut();
		let outputs1 = outputs1.as_mut()[..count].iter_mut();
		let mut iSlow0: i32 = (1.0 - (self.fCheckbox0) as F32) as i32;
		let mut fSlow1: F32 = self.fConst1 * (self.fHslider0) as F32;
		let mut fSlow2: F32 = self.fConst4 * (self.fHslider1) as F32;
		let mut fSlow3: F32 = self.fConst1 * F32::powf(1e+01, 0.05 * (self.fHslider2) as F32);
		let mut fSlow4: F32 = self.fConst1 * (self.fHslider3) as F32;
		let zipped_iterators = inputs0.zip(inputs1).zip(outputs0).zip(outputs1);
		for (((input0, input1), output0), output1) in zipped_iterators {
			self.fRec1[0] = fSlow1 + self.fConst2 * self.fRec1[1];
			let mut fTemp0: F32 = 1.0 / F32::tan(self.fConst3 * self.fRec1[0]);
			let mut fTemp1: F32 = fTemp0 + 1.0;
			self.fRec2[0] = fSlow2 + self.fConst2 * self.fRec2[1];
			let mut fTemp2: F32 = (*input1) as F32;
			let mut fTemp3: F32 = (*input0) as F32;
			let mut fTemp4: F32 = (fTemp3 - fTemp2) * self.fRec2[0];
			self.fRec3[0] = fSlow3 + self.fConst2 * self.fRec3[1];
			let mut fTemp5: F32 = (fTemp3 + fTemp2) * self.fRec3[0];
			let mut fTemp6: F32 = 0.5 * (fTemp5 - fTemp4);
			let mut fTemp7: F32 = (if iSlow0 != 0 {0.0} else {fTemp6});
			self.fVec0[0] = fTemp7;
			let mut fTemp8: F32 = 1.0 - fTemp0;
			self.fRec0[0] = -((self.fRec0[1] * fTemp8 - (fTemp7 + self.fVec0[1])) / fTemp1);
			self.fRec4[0] = fSlow4 + self.fConst2 * self.fRec4[1];
			let mut fTemp9: F32 = 0.5 * (fTemp5 + fTemp4);
			let mut fTemp10: F32 = (if iSlow0 != 0 {0.0} else {fTemp9});
			self.fVec1[0] = fTemp10;
			let mut fTemp11: F32 = 1.0 - 0.5 * self.fRec4[0];
			*output0 = ((if iSlow0 != 0 {fTemp9} else {fTemp11 * fTemp10 + self.fRec4[0] * self.fRec0[0]})) as FaustFloat;
			self.fRec5[0] = -((fTemp8 * self.fRec5[1] - (fTemp10 + self.fVec1[1])) / fTemp1);
			*output1 = ((if iSlow0 != 0 {fTemp6} else {fTemp11 * fTemp7 + self.fRec4[0] * self.fRec5[0]})) as FaustFloat;
			self.fRec1[1] = self.fRec1[0];
			self.fRec2[1] = self.fRec2[0];
			self.fRec3[1] = self.fRec3[0];
			self.fVec0[1] = self.fVec0[0];
			self.fRec0[1] = self.fRec0[0];
			self.fRec4[1] = self.fRec4[0];
			self.fVec1[1] = self.fVec1[0];
			self.fRec5[1] = self.fRec5[0];
		}
		
	}

}

impl FaustDsp for MonitorStereo {
	type T = FaustFloat;
	fn new() -> Self where Self: Sized {
		Self::new()
	}
	fn metadata(&self, m: &mut dyn Meta) {
		self.metadata(m)
	}
	fn get_sample_rate(&self) -> i32 {
		self.get_sample_rate()
	}
	fn get_num_inputs(&self) -> i32 {
		FAUST_INPUTS as i32
	}
	fn get_num_outputs(&self) -> i32 {
		FAUST_OUTPUTS as i32
	}
	fn class_init(sample_rate: i32) where Self: Sized {
		Self::class_init(sample_rate);
	}
	fn instance_reset_params(&mut self) {
		self.instance_reset_params()
	}
	fn instance_clear(&mut self) {
		self.instance_clear()
	}
	fn instance_constants(&mut self, sample_rate: i32) {
		self.instance_constants(sample_rate)
	}
	fn instance_init(&mut self, sample_rate: i32) {
		self.instance_init(sample_rate)
	}
	fn init(&mut self, sample_rate: i32) {
		self.init(sample_rate)
	}
	fn build_user_interface(&self, ui_interface: &mut dyn UI<Self::T>) {
		self.build_user_interface(ui_interface)
	}
	fn build_user_interface_static(ui_interface: &mut dyn UI<Self::T>) where Self: Sized {
		Self::build_user_interface_static(ui_interface);
	}
	fn get_param(&self, param: ParamIndex) -> Option<Self::T> {
		self.get_param(param)
	}
	fn set_param(&mut self, param: ParamIndex, value: Self::T) {
		self.set_param(param, value)
	}
	fn compute(&mut self, count: i32, inputs: &[&[Self::T]], outputs: &mut [&mut [Self::T]]) {
		self.compute(count as usize, inputs, outputs)
	}
}
