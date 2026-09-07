#!/usr/bin/env python3
"""Sample integer registration; requires numpy, pillow, opencv-python-headless."""
import argparse
import json
from pathlib import Path
import cv2
import numpy as np
from PIL import Image


def grayscale(path):
    with Image.open(path) as image:
        return cv2.cvtColor(np.asarray(image.convert('RGB')), cv2.COLOR_RGB2GRAY).astype(np.float32)


def check(master, candidate, regions, radius=6, sigma=1.5):
    if master.shape != candidate.shape:
        raise ValueError('Dimensions must match; do not resize for this check.')
    height, width = master.shape
    pad = radius + int(np.ceil(sigma * 3))
    master_hp = master - cv2.GaussianBlur(master, (0, 0), sigma)
    candidate_hp = candidate - cv2.GaussianBlur(candidate, (0, 0), sigma)
    results = {}
    for name, box in regions.items():
        if len(box) != 4 or any(not isinstance(v, int) for v in box):
            raise ValueError(f'{name}: expected four integer bounds.')
        x0, y0, x1, y1 = box
        if not (pad <= x0 < x1 <= width-pad and pad <= y0 < y1 <= height-pad):
            raise ValueError(f'{name}: invalid bounds or insufficient margin ({pad}px).')
        a = master_hp[y0:y1, x0:x1].ravel().copy()
        a -= a.mean()
        norm_a = np.linalg.norm(a)
        if norm_a < 1e-6:
            results[name] = {'status': 'insufficient_detail'}
            continue
        scores = []
        for dy in range(-radius, radius+1):
            for dx in range(-radius, radius+1):
                b = candidate_hp[y0+dy:y1+dy, x0+dx:x1+dx].ravel().copy()
                b -= b.mean()
                corr = float(np.dot(a, b)/(norm_a*np.linalg.norm(b)+1e-12))
                scores.append((abs(corr), corr, dx, dy))
        scores.sort(reverse=True)
        strength, signed, dx, dy = scores[0]
        results[name] = {'bounds': box, 'offset_px': [dx, dy],
            'correlation': round(strength, 5),
            'polarity': 'same' if signed >= 0 else 'opposite',
            'runner_up_margin': round(strength-scores[1][0], 5),
            'search_boundary': abs(dx) == radius or abs(dy) == radius,
            'status': 'weak_match' if strength < .5 else 'measured'}
    return {'dimensions': [width, height], 'search_radius_px': radius,
        'method': 'absolute normalized correlation of grayscale high-pass patches',
        'caveat': 'Sampled integer offsets; weak or ambiguous matches require visual inspection.',
        'regions': results}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('master', 'candidate', 'regions', 'output'):
        parser.add_argument('--'+name, required=True, type=Path)
    parser.add_argument('--radius', type=int, default=6)
    parser.add_argument('--sigma', type=float, default=1.5)
    args = parser.parse_args()
    if args.radius < 1 or args.sigma <= 0:
        parser.error('radius and sigma must be positive.')
    if args.output.resolve() in {p.resolve() for p in (args.master, args.candidate, args.regions)}:
        parser.error('Output must not overwrite an input.')
    regions = json.loads(args.regions.read_text())
    if not isinstance(regions, dict) or not regions:
        parser.error('Regions must be a nonempty object of named pixel bounds.')
    result = check(grayscale(args.master), grayscale(args.candidate), regions, args.radius, args.sigma)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    zero = sum(r.get('offset_px') == [0, 0] for r in result['regions'].values())
    print(f'{zero}/{len(regions)} sampled patches have zero integer offset. Read confidence fields in {args.output}.')


if __name__ == '__main__':
    main()
