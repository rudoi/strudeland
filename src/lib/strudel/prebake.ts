import * as codemirror from '@strudel/codemirror';
import * as core from '@strudel/core';
import * as draw from '@strudel/draw';
import * as mini from '@strudel/mini';
import * as tonal from '@strudel/tonal';
import * as webaudio from '@strudel/webaudio';
import { registerSynthSounds, registerZZFXSounds } from 'superdough';
import { registerInstrumentOperation } from './instruments';
import { song } from './song';

/** Offline subset of @strudel/repl's prebake. Runs once, before the first evaluate. */
export async function prebake(): Promise<void> {
  await core.evalScope(
    core,
    mini,
    tonal,
    webaudio, // also re-exports all of superdough
    draw,
    codemirror, // provides sliderWithID and registers the widget methods
    { song },
  );
  registerInstrumentOperation();
  registerSynthSounds();
  registerZZFXSounds();
}
