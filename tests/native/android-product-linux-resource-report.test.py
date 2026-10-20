"""Focused offline checks for Canvas2D evidence; no native host actions."""
from copy import deepcopy
from datetime import datetime, timezone
import importlib.util
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location('resource_report', Path(__file__).with_name('android-product-linux-resource-report.py'))
REPORT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REPORT)


def canvas2d_fixture():
    records, driver, phases, frames = REPORT.Tests._full_fixture()
    driver.update(rendererKind='linux-canvas-2d', softwareBackingInferenceOnly=True)
    frames.update(rendererKind='linux-canvas-2d', qualification=False,
                  softwareBackingInferenceOnly=True, measuredPhaseReadback=False)
    for index, view in enumerate(frames['views']):
        view.update(uploadKind='2d-imagedata' if index == 0 else '2d-canvas-copy',
                    fingerprintSamplePixels=64, secondaryBitmapVerified=False,
                    fingerprintProvenance=('ImageData.data-after-successful-putImageData' if index == 0 else
                                           'canonical-primary-input-inherited-after-successful-drawImage'),
                    lastDrawnSourceSequence=view['frames'],
                    contextAttributes=dict(alpha=False, willReadFrequently=True))
    frames['views'][1]['framebuffer'] = [280, 498]
    preflight = deepcopy(frames)
    at = lambda seconds: datetime.fromtimestamp(1700000000 + seconds, timezone.utc).isoformat()
    preflight.update(startedAt=at(-5), endedAt=at(-2), wallElapsed=3,
                     measuredPhaseReadback=True, epoch=8)
    for index, view in enumerate(preflight['views']):
        view.update(frames=90, fps=30, fingerprintedFrames=90, fingerprintTransitions=89,
                    lastDrawnSourceSequence=90, fingerprintProvenance='actual-2d-framebuffer-readback',
                    secondaryBitmapVerified=index == 1)
    driver['changingFramePreflight'] = preflight
    phases.insert(0, dict(phase='workload', event='changing-frame-preflight', at=at(-1), **deepcopy(preflight)))
    return records, driver, phases, frames


class Canvas2DEvidenceTests(unittest.TestCase):
    def test_accepts_actual_bitmap_preflight_and_zero_readback_formal_evidence(self):
        report = REPORT.analyze(*canvas2d_fixture(), expected_source='a' * 64)
        self.assertTrue(report['analysisComplete'])
        self.assertFalse(report['qualification'])
        self.assertAlmostEqual(report['cpuComparison']['summedDeltaMeanCores'], 0.3)
        self.assertEqual(report['applicationPrivateResidentComparison']['meanDeltaBytes'], 5 * REPORT.MIB)
        self.assertTrue(report['frames']['softwareBackingInferenceOnly'])
        self.assertFalse(report['frames']['measuredSecondaryBitmap'])

    def test_legacy_webgl_evidence_remains_readable(self):
        report = REPORT.analyze(*REPORT.Tests._full_fixture())
        self.assertTrue(report['analysisComplete'])
        self.assertEqual(report['frames']['rendererKind'], 'webgl')

    def test_rejects_readbacks_or_bitmap_claims_during_formal_measurement(self):
        for mutation in ('readback', 'bitmap-claim', 'input-provenance', 'canonical-provenance'):
            with self.subTest(mutation=mutation):
                records, driver, phases, frames = canvas2d_fixture()
                if mutation == 'readback':
                    frames['measuredPhaseReadback'] = True
                elif mutation == 'bitmap-claim':
                    frames['views'][1]['secondaryBitmapVerified'] = True
                elif mutation == 'input-provenance':
                    frames['views'][0]['fingerprintProvenance'] = 'actual-2d-framebuffer-readback'
                else:
                    frames['views'][1]['fingerprintProvenance'] = 'ImageData.data-after-successful-putImageData'
                with self.assertRaises(ValueError):
                    REPORT.analyze(records, driver, phases, frames)

    def test_rejects_missing_input_only_or_short_bitmap_preflight(self):
        for mutation in ('missing', 'input-only', 'short', 'wrong-device', 'different-panes', 'journal-mismatch'):
            with self.subTest(mutation=mutation):
                records, driver, phases, frames = canvas2d_fixture()
                preflight = driver['changingFramePreflight']
                if mutation == 'missing':
                    driver.pop('changingFramePreflight')
                elif mutation == 'input-only':
                    preflight['measuredPhaseReadback'] = False
                elif mutation == 'short':
                    preflight['wallElapsed'] = 2
                elif mutation == 'wrong-device':
                    preflight['deviceId'] = 'other-device'
                elif mutation == 'different-panes':
                    preflight['views'][1]['paneId'] = 'other-pane'
                else:
                    phases[0]['views'][1]['secondaryBitmapVerified'] = False
                with self.assertRaises(ValueError):
                    REPORT.analyze(records, driver, phases, frames)

    def test_rejects_inconsistent_canonical_frame_counts_or_dimensions(self):
        for mutation in ('sequence', 'frames', 'small-primary', 'oversized-secondary', 'attributes', 'driver-kind'):
            with self.subTest(mutation=mutation):
                records, driver, phases, frames = canvas2d_fixture()
                if mutation == 'sequence':
                    frames['views'][1]['lastDrawnSourceSequence'] -= 1
                elif mutation == 'frames':
                    view = frames['views'][1]
                    view['frames'] -= 1
                    view['fingerprintedFrames'] = view['lastDrawnSourceSequence'] = view['frames']
                    view['fps'] = view['frames'] / frames['wallElapsed']
                elif mutation == 'small-primary':
                    frames['views'][0]['framebuffer'] = [280, 498]
                elif mutation == 'oversized-secondary':
                    frames['views'][1]['framebuffer'] = [721, 1280]
                elif mutation == 'attributes':
                    frames['views'][0]['contextAttributes']['alpha'] = True
                else:
                    driver['rendererKind'] = 'webgl'
                with self.assertRaises(ValueError):
                    REPORT.analyze(records, driver, phases, frames)


if __name__ == '__main__':
    unittest.main()
