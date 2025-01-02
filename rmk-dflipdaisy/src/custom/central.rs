use embassy_executor::Spawner;
use embassy_time::{Instant, Timer};
use embassy_usb::driver::Driver;
use embedded_hal::digital::{InputPin, OutputPin};
#[cfg(feature = "async_matrix")]
use embedded_hal_async::digital::Wait;
#[cfg(any(feature = "_nrf_ble", not(feature = "_no_external_storage")))]
use embedded_storage_async::nor_flash::NorFlash;

use rmk::action::KeyAction;
#[cfg(feature = "_nrf_ble")]
use rmk::ble::nrf::initialize_nrf_ble_keyboard_and_run;
use rmk::config::RmkConfig;
#[cfg(not(feature = "rapid_debouncer"))]
use rmk::debounce::default_bouncer::DefaultDebouncer;
#[cfg(feature = "rapid_debouncer")]
use rmk::debounce::fast_debouncer::RapidDebouncer;
use rmk::debounce::{DebounceState, DebouncerTrait};
use rmk::keyboard::{key_event_channel, KeyEvent};
use rmk::matrix::{KeyState, MatrixTrait};
use rmk::split::central::initialize_usb_split_central_and_run;


/// Run RMK split central keyboard service. This function should never return.
///
/// # Arguments
///
/// * `input_pins` - input gpio pins, if `async_matrix` is enabled, the input pins should implement `embedded_hal_async::digital::Wait` trait
/// * `output_pins` - output gpio pins
/// * `usb_driver` - (optional) embassy usb driver instance. Some microcontrollers would enable the `_no_usb` feature implicitly, which eliminates this argument
/// * `flash` - (optional) flash storage, which is used for storing keymap and keyboard configs. Some microcontrollers would enable the `_no_external_storage` feature implicitly, which eliminates this argument
/// * `default_keymap` - default keymap definition
/// * `keyboard_config` - other configurations of the keyboard, check [RmkConfig] struct for details
/// * `central_addr` - (optional) central's BLE static address. This argument is enabled only for nRF BLE split central now
/// * `spawner`: (optional) embassy spawner used to spawn async tasks. This argument is enabled for non-esp microcontrollers
#[allow(unused_variables)]
#[allow(unreachable_code)]
pub async fn run_rmk_split_central<
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    Out: OutputPin,
    #[cfg(not(feature = "_no_usb"))] D: Driver<'static>,
    #[cfg(not(feature = "_no_external_storage"))] F: NorFlash,
    const TOTAL_ROW: usize,
    const TOTAL_COL: usize,
    const CENTRAL_ROW: usize,
    const CENTRAL_COL: usize,
    const CENTRAL_ROW_OFFSET: usize,
    const CENTRAL_COL_OFFSET: usize,
    const NUM_LAYER: usize,
>(
    #[cfg(feature = "col2row")] input_pins: [In; CENTRAL_ROW],
    #[cfg(not(feature = "col2row"))] input_pins: [In; CENTRAL_COL],
    #[cfg(feature = "col2row")] output_pins: [Out; CENTRAL_COL],
    #[cfg(not(feature = "col2row"))] output_pins: [Out; CENTRAL_ROW],
    #[cfg(not(feature = "_no_usb"))] usb_driver: D,
    #[cfg(not(feature = "_no_external_storage"))] flash: F,
    default_keymap: &mut [[[KeyAction; TOTAL_COL]; TOTAL_ROW]; NUM_LAYER],
    keyboard_config: RmkConfig<'static, Out>,
    #[cfg(feature = "_nrf_ble")] central_addr: [u8; 6],
    #[cfg(not(feature = "_esp_ble"))] spawner: Spawner,
) -> ! {
    // Create the debouncer, use COL2ROW by default
    #[cfg(all(feature = "col2row", feature = "rapid_debouncer"))]
    let debouncer: RapidDebouncer<CENTRAL_ROW, CENTRAL_COL> = RapidDebouncer::new();
    #[cfg(all(not(feature = "col2row"), feature = "rapid_debouncer"))]
    let debouncer: RapidDebouncer<CENTRAL_COL, CENTRAL_ROW> = RapidDebouncer::new();
    #[cfg(all(feature = "col2row", not(feature = "rapid_debouncer")))]
    let debouncer: DefaultDebouncer<CENTRAL_ROW, CENTRAL_COL> = DefaultDebouncer::new();
    #[cfg(all(not(feature = "col2row"), not(feature = "rapid_debouncer")))]
    let debouncer: DefaultDebouncer<CENTRAL_COL, CENTRAL_ROW> = DefaultDebouncer::new();

    // Keyboard matrix, use COL2ROW by default
    #[cfg(feature = "col2row")]
    let matrix = CentralMatrix::<
        In,
        Out,
        _,
        CENTRAL_ROW_OFFSET,
        CENTRAL_COL_OFFSET,
        CENTRAL_ROW,
        CENTRAL_COL,
    >::new(input_pins, output_pins, debouncer);
    #[cfg(not(feature = "col2row"))]
    let matrix = CentralMatrix::<
        In,
        Out,
        _,
        CENTRAL_ROW_OFFSET,
        CENTRAL_COL_OFFSET,
        CENTRAL_COL,
        CENTRAL_ROW,
    >::new(input_pins, output_pins, debouncer);

    #[cfg(feature = "_nrf_ble")]
    let fut = initialize_nrf_ble_keyboard_and_run::<_, _, D, TOTAL_ROW, TOTAL_COL, NUM_LAYER>(
        matrix,
        usb_driver,
        default_keymap,
        keyboard_config,
        Some(central_addr),
        spawner,
    )
    .await;

    #[cfg(not(any(feature = "_nrf_ble", feature = "_esp_ble")))]
    let fut = initialize_usb_split_central_and_run::<_, _, D, F, TOTAL_ROW, TOTAL_COL, NUM_LAYER>(
        matrix,
        usb_driver,
        flash,
        default_keymap,
        keyboard_config,
    )
    .await;

    fut
}



/// Matrix is the physical pcb layout of the keyboard matrix.
pub(crate) struct CentralMatrix<
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    Out: OutputPin,
    D: DebouncerTrait,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
    const INPUT_PIN_NUM: usize,
    const OUTPUT_PIN_NUM: usize,
> {
    /// Input pins of the pcb matrix
    input_pins: [In; INPUT_PIN_NUM],
    /// Output pins of the pcb matrix
    output_pins: [Out; OUTPUT_PIN_NUM],
    /// Debouncer
    debouncer: D,
    /// Key state matrix
    key_states: [[KeyState; INPUT_PIN_NUM]; OUTPUT_PIN_NUM],
    /// Start scanning
    scan_start: Option<Instant>,
}

impl<
        #[cfg(feature = "async_matrix")] In: Wait + InputPin,
        #[cfg(not(feature = "async_matrix"))] In: InputPin,
        Out: OutputPin,
        D: DebouncerTrait,
        const ROW_OFFSET: usize,
        const COL_OFFSET: usize,
        const INPUT_PIN_NUM: usize,
        const OUTPUT_PIN_NUM: usize,
    > MatrixTrait
    for CentralMatrix<In, Out, D, ROW_OFFSET, COL_OFFSET, INPUT_PIN_NUM, OUTPUT_PIN_NUM>
{
    #[cfg(feature = "col2row")]
    const ROW: usize = INPUT_PIN_NUM;
    #[cfg(feature = "col2row")]
    const COL: usize = OUTPUT_PIN_NUM;
    #[cfg(not(feature = "col2row"))]
    const ROW: usize = OUTPUT_PIN_NUM;
    #[cfg(not(feature = "col2row"))]
    const COL: usize = INPUT_PIN_NUM;

    async fn scan(&mut self) {
        defmt::info!("Central matrix scanning");
        loop {
            #[cfg(feature = "async_matrix")]
            self.wait_for_key().await;

            // Scan matrix and send report
            for (out_idx, out_pin) in self.output_pins.iter_mut().enumerate() {
                // Pull up output pin, wait 1us ensuring the change comes into effect
                out_pin.set_high().ok();
                Timer::after_micros(1).await;
                for (in_idx, in_pin) in self.input_pins.iter_mut().enumerate() {
                    // Check input pins and debounce
                    let debounce_state = self.debouncer.detect_change_with_debounce(
                        in_idx,
                        out_idx,
                        in_pin.is_high().ok().unwrap_or_default(),
                        &self.key_states[out_idx][in_idx],
                    );

                    match debounce_state {
                        DebounceState::Debounced => {
                            self.key_states[out_idx][in_idx].toggle_pressed();
                            #[cfg(feature = "col2row")]
                            let (row, col, key_state) = (
                                (in_idx + ROW_OFFSET) as u8,
                                (out_idx + COL_OFFSET) as u8,
                                self.key_states[out_idx][in_idx],
                            );
                            #[cfg(not(feature = "col2row"))]
                            let (row, col, key_state) = (
                                (out_idx + ROW_OFFSET) as u8,
                                (in_idx + COL_OFFSET) as u8,
                                self.key_states[out_idx][in_idx],
                            );

                            key_event_channel
                                .send(KeyEvent {
                                    row,
                                    col,
                                    pressed: key_state.pressed,
                                })
                                .await;
                        }
                        _ => (),
                    }

                    // If there's key still pressed, always refresh the self.scan_start
                    #[cfg(feature = "async_matrix")]
                    if self.key_states[out_idx][in_idx].pressed {
                        self.scan_start = Some(Instant::now());
                    }
                }
                out_pin.set_low().ok();
            }

            embassy_time::Timer::after_micros(100).await;
        }
    }

    fn get_key_state(&mut self, row: usize, col: usize) -> KeyState {
        self.key_states[row][col]
    }

    fn update_key_state(&mut self, row: usize, col: usize, f: impl FnOnce(&mut KeyState)) {
        f(&mut self.key_states[row][col]);
    }

    #[cfg(feature = "async_matrix")]
    async fn wait_for_key(&mut self) {
        use defmt::info;
        use embassy_futures::select::select_slice;
        use heapless::Vec;

        if let Some(start_time) = self.scan_start {
            // If not key over 2 secs, wait for interupt in next loop
            if start_time.elapsed().as_secs() < 1 {
                return;
            } else {
                self.scan_start = None;
            }
        }
        // First, set all output pin to high
        for out in self.output_pins.iter_mut() {
            out.set_high().ok();
        }
        Timer::after_micros(1).await;
        info!("Waiting for high");
        let mut futs: Vec<_, INPUT_PIN_NUM> = self
            .input_pins
            .iter_mut()
            .map(|input_pin| input_pin.wait_for_high())
            .collect();
        let _ = select_slice(futs.as_mut_slice()).await;

        // Set all output pins back to low
        for out in self.output_pins.iter_mut() {
            out.set_low().ok();
        }

        self.scan_start = Some(Instant::now());
    }
}

impl<
        #[cfg(feature = "async_matrix")] In: Wait + InputPin,
        #[cfg(not(feature = "async_matrix"))] In: InputPin,
        Out: OutputPin,
        D: DebouncerTrait,
        const ROW_OFFSET: usize,
        const COL_OFFSET: usize,
        const INPUT_PIN_NUM: usize,
        const OUTPUT_PIN_NUM: usize,
    > CentralMatrix<In, Out, D, ROW_OFFSET, COL_OFFSET, INPUT_PIN_NUM, OUTPUT_PIN_NUM>
{
    /// Initialization of central
    pub(crate) fn new(
        input_pins: [In; INPUT_PIN_NUM],
        output_pins: [Out; OUTPUT_PIN_NUM],
        debouncer: D,
    ) -> Self {
        CentralMatrix {
            input_pins,
            output_pins,
            debouncer,
            key_states: [[KeyState::default(); INPUT_PIN_NUM]; OUTPUT_PIN_NUM],
            scan_start: None,
        }
    }
}


