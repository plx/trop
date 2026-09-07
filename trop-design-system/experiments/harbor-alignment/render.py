from pathlib import Path
import argparse
import hashlib
import json
import shutil
import numpy as np
import cv2
from PIL import Image, ImageDraw

RECIPE=Path(__file__).resolve().parent
parser=argparse.ArgumentParser(description='Reproduce the harbor pair from one geometry master and separate lighting.')
parser.add_argument('--day',type=Path,default=RECIPE.parent.parent/'assets/harbor-hero-light.png')
parser.add_argument('--night',type=Path,default=RECIPE.parent.parent/'assets/harbor-hero-dark.png')
parser.add_argument('--output',type=Path,required=True,help='A new output directory; existing nonempty directories are refused.')
args=parser.parse_args()
ROOT=args.output.resolve()
if ROOT.exists() and any(ROOT.iterdir()):parser.error('Output directory must be new or empty.')
expected={'day':'2cf9ff3b0e80214f3ea9f602794eff9fef38b895773c22d7a941c23d3eb82b9c','night':'b84c90aa724af042a481ede4020ec91d8079e643233d4a9473b9f690821cac5f'}
for name in expected:
    if hashlib.sha256(getattr(args,name).read_bytes()).hexdigest()!=expected[name]:
        parser.error(f'{name} differs from the original used by this scene-specific recipe; provide the original with --{name}.')
day=np.asarray(Image.open(args.day).convert('RGB'), dtype=np.float32)
night=np.asarray(Image.open(args.night).convert('RGB'), dtype=np.float32)
if day.shape!=night.shape or day.shape!=(958,1642,3):parser.error('Expected matching 1642 × 958 RGB images.')
ROOT.mkdir(parents=True,exist_ok=True)
H,W=day.shape[:2]
def blur(a,s):return cv2.GaussianBlur(a,(0,0),s)
def save(name,a):Image.fromarray(np.clip(a,0,255).round().astype(np.uint8)).save(ROOT/name)
def polygon(points,soft=2):
    m=np.zeros((H,W),np.float32);cv2.fillPoly(m,[np.array(points,np.int32)],1)
    return blur(m,soft) if soft else m
d=cv2.cvtColor(day,cv2.COLOR_RGB2GRAY)

# All surface regions follow the daytime master. Feathered interior masks
# change tone only; they do not introduce replacement linework.
surfaces={
'quay':[(685,958),(1280,769),(1476,704),(1642,647),(1642,958)],
'bollard':[(1274,699),(1288,693),(1312,689),(1345,692),(1370,699),(1385,707),(1383,719),(1365,732),(1371,798),(1397,808),(1415,830),(1402,843),(1360,857),(1307,854),(1276,844),(1247,824),(1258,802),(1287,797),(1291,730),(1277,723),(1272,713)],
'small-bollard':[(1496,674),(1528,672),(1541,680),(1535,690),(1529,695),(1534,718),(1545,725),(1538,735),(1487,735),(1489,724),(1504,719),(1502,695),(1497,690)],
'far-bollard':[(1585,653),(1606,652),(1614,658),(1609,665),(1607,688),(1614,693),(1608,700),(1580,701),(1581,692),(1590,687),(1590,666),(1584,662)],
'ship':[(1068,580),(1105,576),(1111,507),(1094,499),(1090,489),(1102,483),(1143,481),(1143,473),(1161,474),(1162,481),(1193,485),(1195,493),(1229,498),(1230,557),(1289,565),(1324,574),(1314,592),(1281,636),(1286,646),(1199,646),(1099,637),(1087,626)],
'containers':[(1119,542),(1157,526),(1158,517),(1196,508),(1220,509),(1223,516),(1261,516),(1263,535),(1281,537),(1281,563),(1214,563),(1178,577),(1118,587)],
'dock-facade':[(1310,624),(1510,632),(1642,646),(1642,661),(1480,650),(1297,640)],
'stacked-containers':[(1409,557),(1440,553),(1442,540),(1460,544),(1461,549),(1490,550),(1491,535),(1527,537),(1527,557),(1546,555),(1548,541),(1558,543),(1557,535),(1587,537),(1587,519),(1612,520),(1615,539),(1642,540),(1642,621),(1412,618)],
}
master_ink=np.clip((246-d)/140,0,1)
# Sparse exposed engraving, e.g. rigging and buoy, changes ink polarity.
outside=master_ink*66
# Opaque surfaces retain the master's shading polarity and fine hatching.
inside=(np.clip((d-65)/180,0,1)**2)*100
mask=np.maximum.reduce([polygon(p,2) for p in surfaces.values()])
value=outside*(1-mask)+inside*mask
base=np.array([7,19,32],np.float32)
out=base+value[...,None]*np.array([1.10,1,.86],np.float32)
save('night-shared-base.png',out)
save('surface-mask.png',np.repeat(mask[...,None]*255,3,axis=2))

# Tone support from the donor, blurred sufficiently to carry only broad
# lighting envelopes. Restrict this first pass to existing physical surfaces.
for sigma in [6,8,16]:
    target=blur(night,sigma)
    source=blur(out,sigma)
    # Multiplicative broad illumination avoids copying donor outlines.
    gain=np.clip((target-base+3)/(source-base+3),.25,2.5)
    gain=blur(gain,sigma)
    result=base+(out-base)*gain
    save(f'night-shared-toned-{sigma}.png',result)

json.dump(surfaces,open(ROOT/'surface-regions.json','w'),indent=2)

# The original night is the lighting donor because it drifts less in mood
# than the whole-scene regeneration. ImageGen remains available for isolated
# effects; no donor structure is allowed into the master geometry layer.
Y,X=np.mgrid[:H,:W].astype(np.float32)
final=np.asarray(Image.open(ROOT/'night-shared-toned-6.png'),dtype=np.float32)
detail=final-blur(final,2)
target_detail=night-blur(night,2)
detail_rms=np.sqrt(blur(np.mean(detail**2,axis=2),8)+1)
target_rms=np.sqrt(blur(np.mean(target_detail**2,axis=2),8)+1)
contrast_gain=blur(np.clip(target_rms/detail_rms,.65,2.6),8)
final+=detail*(contrast_gain-1)[...,None]
struct=final.copy()

# Sparse stars extracted from feature-free ImageGen sky; float32 values
# are retained losslessly so reproduction does not invoke ImageGen again.
with np.load(RECIPE/'generated-stars.npz',allow_pickle=False) as data:
    stars=data['stars']
if stars.shape!=day.shape:raise ValueError('Star layer has the wrong dimensions.')
final+=stars

# Moon enhancement is modulated by the original crescent pixels, so even
# its outline remains registered. This is separate from the base palette.
moon_roi=((X>1013)&(X<1072)&(Y>76)&(Y<135)).astype(np.float32)
moon_ink=np.clip((238-d)/40,0,1)*moon_roi
moon=moon_ink[...,None]*np.array([119,111,83])
moon+=blur(moon_ink,8)[...,None]*np.array([10,10,9])
final+=moon

# Extract luminous content only: warm lamp cores and their local light.
# Thresholds exclude the donor's blue-grey engraving and navy substrate.
lum=cv2.cvtColor(night,cv2.COLOR_RGB2GRAY)
warm=np.maximum(night[...,0]-night[...,2]-10,0)
emission=np.minimum(np.maximum((lum-65)/75,0),np.maximum(warm/30,0))
emission=np.clip(emission,0,1)
lamp_roi=((X>920)&(Y>145)&(Y<627))|((X>475)&(X<505)&(Y>583)&(Y<613))|((X>691)&(X<722)&(Y>638)&(Y<666))
emission*=lamp_roi
core=((lum>165)&lamp_roi).astype(np.uint8)
count,labels,stats,centroids=cv2.connectedComponentsWithStats(core)
lights=[]
for i in range(1,count):
    if stats[i,cv2.CC_STAT_AREA]<4: continue
    cx,cy=centroids[i]
    if stats[i,cv2.CC_STAT_AREA]>70: continue
    lights.append([round(float(cx),2),round(float(cy),2),int(stats[i,cv2.CC_STAT_AREA])])
json.dump(lights,open(ROOT/'detected-lights.json','w'),indent=2)
light_layer=np.zeros_like(final)
for cx,cy,area in lights:
    # Source windows need to illuminate master windows, not neighboring wall.
    if 1105<cx<1195 and 485<cy<538:
        xx=int(round(cx)); yy=int(round(cy))
        tile=d[yy-4:yy+5,xx-4:xx+5]
        # Local dark-pixel centroid restricted to the window-sized neighborhood.
        weights=np.maximum(185-tile,0)
        if weights.sum()>0:
            sy,sx=np.mgrid[-4:5,-4:5]
            cx+=float((weights*sx).sum()/weights.sum())
            cy+=float((weights*sy).sum()/weights.sum())
    # Primary navigation fixtures mapped to their native daytime coordinates.
    if 480<cx<501 and 585<cy<610:cx,cy=489,600
    if 696<cx<718 and 640<cy<663:cx,cy=707,650
    if 1325<cx<1345 and cy<174:cx,cy=1339,169
    radius=np.clip(np.sqrt(area)/2,.5,1.4)
    r2=(X-cx)**2+(Y-cy)**2
    # Small warm point cores, tight glow, and restrained broad halo.
    energy=np.clip(np.sqrt(area),1.2,3)
    color=np.array([175,155,114])
    if (cx<750) or (cy<190) or (cx<1100 and cy<500) or (cx>1600 and cy<310):
        color=np.array([152,181,162])
    light_layer+=np.exp(-r2/(2*radius**2))[...,None]*color
    light_layer+=np.exp(-r2/14)[...,None]*np.array([12,9,5])*energy
    light_layer+=np.exp(-r2/110)[...,None]*np.array([2.2,1.8,1.0])*energy
final+=light_layer

# Distant city lights are retained in the shoreline band, intersected with
# master shoreline detail so no donor hills or buildings can enter the image.
city_roi=((X<930)&(Y>600)&(Y<630)).astype(np.float32)
city_warm=np.clip((night[...,0]-night[...,2]-8)/18,0,1)*city_roi
city_signal=np.maximum(night-np.array([12,23,37]),0)*city_warm[...,None]
city_signal*=np.clip((246-d)/18,0,1)[...,None]
final+=city_signal

# Water is an intentional variable region. Its lighting is transferred as
# a color envelope, then expressed through the master's exact water marks.
# No donor buoy, ship edge, quay edge, or bollard is copied.
water_mask=polygon([(0,633),(1064,633),(1100,650),(1280,658),(1463,674),(1458,694),(1225,775),(660,958),(0,958)],3)
for key in ['bollard','small-bollard','far-bollard']:
    water_mask*=1-polygon(surfaces[key],5)
buoy_mask=polygon([(679,647),(720,644),(741,743),(726,761),(682,757),(675,734)],4)
water_mask*=1-buoy_mask
color_envelope=np.maximum(blur(night,3)-np.array([12,23,37]),0)
warm_water=np.maximum(color_envelope[...,0]-color_envelope[...,2]*1.05,0)
warm_water=blur(warm_water,2)*water_mask
water_marks=np.clip((246-d)/60,0,1)
reflections=warm_water[...,None]*np.array([1.9,1.4,.8])*water_marks[...,None]
final+=reflections

save('harbor-hero-dark.png',final)
shutil.copy2(args.day,ROOT/'harbor-hero-light.png')
save('lighting-only.png',stars+moon+light_layer+reflections)
save('water-mask.png',np.repeat(water_mask[...,None]*255,3,axis=2))
print(f'Wrote 1642 × 958 pair and diagnostic layers to {ROOT}')
