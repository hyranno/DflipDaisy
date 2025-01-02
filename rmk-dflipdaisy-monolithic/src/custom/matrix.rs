use rmk::{
  debounce::{DebounceState, DebouncerTrait},
  keyboard::{key_event_channel, KeyEvent},
  matrix::{MatrixTrait, KeyState},
};
use embassy_time::{Instant, Timer};
use embedded_hal::digital::{InputPin, OutputPin};
#[cfg(feature = "async_matrix")]
use {embassy_futures::select::select_slice, embedded_hal_async::digital::Wait, heapless::Vec};


// TODO: customize later
/// Matrix is the physical pcb layout of the keyboard matrix.
pub(crate) struct Matrix<
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    Out: OutputPin,
    D: DebouncerTrait,
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
        #[cfg(not(feature = "async_matrix"))] In: InputPin,
        #[cfg(feature = "async_matrix")] In: Wait + InputPin,
        Out: OutputPin,
        D: DebouncerTrait,
        const INPUT_PIN_NUM: usize,
        const OUTPUT_PIN_NUM: usize,
    > Matrix<In, Out, D, INPUT_PIN_NUM, OUTPUT_PIN_NUM>
{
    /// Create a matrix from input and output pins.
    pub(crate) fn new(
        input_pins: [In; INPUT_PIN_NUM],
        output_pins: [Out; OUTPUT_PIN_NUM],
        debouncer: D,
    ) -> Self {
        Matrix {
            input_pins,
            output_pins,
            debouncer,
            key_states: [[KeyState::new(); INPUT_PIN_NUM]; OUTPUT_PIN_NUM],
            scan_start: None,
        }
    }
}

impl<
        #[cfg(not(feature = "async_matrix"))] In: InputPin,
        #[cfg(feature = "async_matrix")] In: Wait + InputPin,
        Out: OutputPin,
        D: DebouncerTrait,
        const INPUT_PIN_NUM: usize,
        const OUTPUT_PIN_NUM: usize,
    > MatrixTrait for Matrix<In, Out, D, INPUT_PIN_NUM, OUTPUT_PIN_NUM>
{
    #[cfg(feature = "col2row")]
    const ROW: usize = INPUT_PIN_NUM;
    #[cfg(feature = "col2row")]
    const COL: usize = OUTPUT_PIN_NUM;
    #[cfg(not(feature = "col2row"))]
    const ROW: usize = OUTPUT_PIN_NUM;
    #[cfg(not(feature = "col2row"))]
    const COL: usize = INPUT_PIN_NUM;

    #[cfg(feature = "async_matrix")]
    async fn wait_for_key(&mut self) {
        if let Some(start_time) = self.scan_start {
            // If no key press over 1ms, stop scanning and wait for interupt
            if start_time.elapsed().as_millis() <= 1 {
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

    /// Do matrix scanning, the result is stored in matrix's key_state field.
    async fn scan(&mut self) {
        defmt::info!("Matrix scanning");
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
                            let (row, col, key_state) =
                                (in_idx, out_idx, self.key_states[out_idx][in_idx]);
                            #[cfg(not(feature = "col2row"))]
                            let (row, col, key_state) =
                                (out_idx, in_idx, self.key_states[out_idx][in_idx]);

                            key_event_channel
                                .send(KeyEvent {
                                    row: row as u8,
                                    col: col as u8,
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

    /// Read key state at position (row, col)
    fn get_key_state(&mut self, row: usize, col: usize) -> KeyState {
        // COL2ROW
        #[cfg(feature = "col2row")]
        return self.key_states[col][row];

        // ROW2COL
        #[cfg(not(feature = "col2row"))]
        return self.key_states[row][col];
    }

    fn update_key_state(&mut self, row: usize, col: usize, f: impl FnOnce(&mut KeyState)) {
        // COL2ROW
        #[cfg(feature = "col2row")]
        f(&mut self.key_states[col][row]);

        // ROW2COL
        #[cfg(not(feature = "col2row"))]
        f(&mut self.key_states[row][col]);
    }
}