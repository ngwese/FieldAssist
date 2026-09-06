/* ------------------------------------------------------------
license: "MIT"
name: "MonitorFoa"
version: "1.0"
Code generated with Faust 2.85.9 (https://faust.grame.fr)
Compilation options: -lang rust -fpga-mem-th 4 -ct 1 -cn MonitorFoa -es 1 -mcd 16 -mdd 1024 -mdy 33 -single -ftz 0
------------------------------------------------------------ */

#[repr(C)]
pub struct MonitorFoa {
	fSampleRate: i32,
	fConst0: F32,
	fConst1: F32,
	fHslider0: FaustFloat,
	fRec0: [F32;2],
	fHslider1: FaustFloat,
	fRec1: [F32;2],
}


pub const FAUST_INPUTS: usize = 4;
pub const FAUST_OUTPUTS: usize = 2;
pub const FAUST_ACTIVES: usize = 2;
pub const FAUST_PASSIVES: usize = 0;

impl MonitorFoa {
		
	pub fn new() -> MonitorFoa { 
		MonitorFoa {
			fSampleRate: 0,
			fConst0: 0.0,
			fConst1: 0.0,
			fHslider0: 0.0,
			fRec0: [0.0;2],
			fHslider1: 0.0,
			fRec1: [0.0;2],
		}
	}
	pub fn metadata(&self, m: &mut dyn Meta) { 
		m.declare("basics.lib/name", r"Faust Basic Element Library");
		m.declare("basics.lib/version", r"1.22.0");
		m.declare("compile_options", r"-lang rust -fpga-mem-th 4 -ct 1 -cn MonitorFoa -es 1 -mcd 16 -mdd 1024 -mdy 33 -single -ftz 0");
		m.declare("filename", r"monitor_foa.dsp");
		m.declare("license", r"MIT");
		m.declare("maths.lib/author", r"GRAME");
		m.declare("maths.lib/copyright", r"GRAME");
		m.declare("maths.lib/license", r"LGPL with exception");
		m.declare("maths.lib/name", r"Faust Math Library");
		m.declare("maths.lib/version", r"2.9.0");
		m.declare("name", r"MonitorFoa");
		m.declare("platform.lib/name", r"Generic Platform Library");
		m.declare("platform.lib/version", r"1.3.0");
		m.declare("signals.lib/name", r"Faust Routing Library");
		m.declare("signals.lib/version", r"1.6.0");
		m.declare("version", r"1.0");
	}

	pub fn get_sample_rate(&self) -> i32 { self.fSampleRate as i32}
	
	pub fn class_init(sample_rate: i32) {
		// Obtaining locks on 0 static var(s)
	}
	pub fn instance_reset_params(&mut self) {
		self.fHslider0 = (0.0) as FaustFloat;
		self.fHslider1 = (0.0) as FaustFloat;
	}
	pub fn instance_clear(&mut self) {
		for l0 in 0..2 {
			self.fRec0[l0 as usize] = 0.0;
		}
		for l1 in 0..2 {
			self.fRec1[l1 as usize] = 0.0;
		}
	}
	pub fn instance_constants(&mut self, sample_rate: i32) {
		// Obtaining locks on 0 static var(s)
		self.fSampleRate = sample_rate;
		self.fConst0 = 44.1 / F32::min(1.92e+05, F32::max(1.0, (self.fSampleRate) as F32));
		self.fConst1 = 1.0 - self.fConst0;
	}
	pub fn instance_init(&mut self, sample_rate: i32) {
		self.instance_constants(sample_rate);
		self.instance_reset_params();
		self.instance_clear();
	}
	pub fn init(&mut self, sample_rate: i32) {
		MonitorFoa::class_init(sample_rate);
		self.instance_init(sample_rate);
	}
	
	pub fn build_user_interface(&self, ui_interface: &mut dyn UI<FaustFloat>) {
		Self::build_user_interface_static(ui_interface);
	}
	
	pub fn build_user_interface_static(ui_interface: &mut dyn UI<FaustFloat>) {
		ui_interface.open_vertical_box("MonitorFoa");
		ui_interface.declare(Some(ParamIndex(0)), "unit", "deg");
		ui_interface.add_horizontal_slider("Ambisonics/Yaw", ParamIndex(0), 0.0, -1.8e+02, 1.8e+02, 0.1);
		ui_interface.declare(Some(ParamIndex(1)), "style", "knob");
		ui_interface.declare(Some(ParamIndex(1)), "unit", "dB");
		ui_interface.add_horizontal_slider("Monitor/Gain", ParamIndex(1), 0.0, -9e+01, 12.0, 0.1);
		ui_interface.close_box();
	}
	
	pub fn get_param(&self, param: ParamIndex) -> Option<FaustFloat> {
		match param.0 {
			0 => Some(self.fHslider0),
			1 => Some(self.fHslider1),
			_ => None,
		}
	}
	
	pub fn set_param(&mut self, param: ParamIndex, value: FaustFloat) {
		match param.0 {
			0 => { self.fHslider0 = value }
			1 => { self.fHslider1 = value }
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
		let [inputs0, inputs1, inputs2, inputs3, .. ] = inputs.as_ref() else { panic!("wrong number of input buffers"); };
		let inputs0 = inputs0.as_ref()[..count].iter();
		let inputs1 = inputs1.as_ref()[..count].iter();
		let inputs2 = inputs2.as_ref()[..count].iter();
		let inputs3 = inputs3.as_ref()[..count].iter();
		let [outputs0, outputs1, .. ] = outputs.as_mut() else { panic!("wrong number of output buffers"); };
		let outputs0 = outputs0.as_mut()[..count].iter_mut();
		let outputs1 = outputs1.as_mut()[..count].iter_mut();
		let mut fSlow0: F32 = self.fConst0 * (self.fHslider0) as F32;
		let mut fSlow1: F32 = self.fConst0 * F32::powf(1e+01, 0.05 * (self.fHslider1) as F32);
		let zipped_iterators = inputs0.zip(inputs1).zip(inputs2).zip(inputs3).zip(outputs0).zip(outputs1);
		for (((((input0, input1), input2), input3), output0), output1) in zipped_iterators {
			self.fRec0[0] = fSlow0 + self.fConst1 * self.fRec0[1];
			let mut fTemp0: F32 = 0.017453292 * self.fRec0[0];
			let mut fTemp1: F32 = F32::sin(fTemp0);
			let mut fTemp2: F32 = (*input3) as F32;
			let mut fTemp3: F32 = F32::cos(fTemp0);
			let mut fTemp4: F32 = (*input1) as F32;
			let mut fTemp5: F32 = 0.5 * (fTemp4 * fTemp3 - fTemp2 * fTemp1);
			let mut fTemp6: F32 = 0.70710677 * (*input0) as F32 + 0.8660254 * (fTemp2 * fTemp3 + fTemp4 * fTemp1);
			self.fRec1[0] = fSlow1 + self.fConst1 * self.fRec1[1];
			*output0 = (self.fRec1[0] * (fTemp6 + fTemp5)) as FaustFloat;
			*output1 = (self.fRec1[0] * (fTemp6 - fTemp5)) as FaustFloat;
			self.fRec0[1] = self.fRec0[0];
			self.fRec1[1] = self.fRec1[0];
		}
		
	}

}

impl FaustDsp for MonitorFoa {
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
