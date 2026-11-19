// Port of CORE-MATH (018fa97cbdacf59924b899a9dfa422d532cb2bb7) src/binary64/exp/exp.c, cr_exp.
// MIT; see LICENSE-core-math.
//
// Correctly rounded exponential function for binary64 values.
// Copyright (c) 2022-2026 Alexei Sibidanov <sibid@uvic.ca>.
// This file is part of the CORE-MATH project (https://core-math.gitlabpages.inria.fr/).
//
// Ported for round-to-nearest only, without errno or floating-point exception flags. The C is
// built with `-ffp-contract=off`, so only the explicit `__builtin_fma` calls are fused
// (`f64::mul_add`); every other `a * b + c` is a rounded product then a rounded sum.

#[inline(always)]
fn fasttwosum(x: f64, y: f64, e: &mut f64) -> f64 {
    let s = x + y;
    let z = s - x;
    *e = y - z;
    s
}

#[inline(always)]
fn fastsum(xh: f64, xl: f64, yh: f64, yl: f64, e: &mut f64) -> f64 {
    let mut sl = 0.0;
    let sh = fasttwosum(xh, yh, &mut sl);
    *e = (xl + yl) + sl;
    sh
}

#[inline(always)]
fn muldd(xh: f64, xl: f64, ch: f64, cl: f64, l: &mut f64) -> f64 {
    let ahhh = ch * xh;
    *l = (ch * xl + cl * xh) + ch.mul_add(xh, -ahhh);
    ahhh
}

/* This routine implements Algorithm 4 from
"Extended-Precision FMA under Parameterized Double-Word Overlap:
Tight Error Bounds and Examples by Claude-Pierre Jeannerod, Mioara Joldes,
Nicolas Louvet, Jean-Michel Muller, published in the proceedings of
Arith 2026, https://inria.hal.science/hal-05517451.

With respect to the original algorithm, g = __builtin_fma (ah, bl, f) is
replaced by g = ah * bl + f, and *l = __builtin_fma (al, bh, g) by
*l = al * bh + g (not fused under -ffp-contract=off). */
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn fast_fma_dw(ah: f64, al: f64, bh: f64, bl: f64, ch: f64, cl: f64, l: &mut f64) -> f64 {
    let dh = ah.mul_add(bh, ch);
    let t = ch - dh;
    let e = ah.mul_add(bh, t);
    let f = e + cl;
    // let g = ah.mul_add(bl, f); // original algorithm
    let g = ah * bl + f;
    // *l = al.mul_add(bh, g); // original algorithm
    *l = al * bh + g;
    // *l is dl in the paper
    dh
}

#[inline(always)]
fn opolydd(xh: f64, xl: f64, c: &[[f64; 2]], l: &mut f64) -> f64 {
    let mut i = c.len() - 1;
    let mut ch = c[i][0];
    let mut cl = c[i][1];
    while i > 0 {
        i -= 1;
        let cl0 = cl;
        ch = fast_fma_dw(xh, xl, ch, cl0, c[i][0], c[i][1], &mut cl);
    }
    *l = cl;
    ch
}

#[inline(always)]
fn as_ldexp(x: f64, i: i64) -> f64 {
    f64::from_bits(x.to_bits().wrapping_add((i as u64) << 52))
}

// sets the exponent of a binary64 number to 0 (subnormal range)
#[inline(always)]
fn as_todenormal(x: f64) -> f64 {
    f64::from_bits(x.to_bits() & (!0u64 >> 12))
}

/* list of exceptional cases for which the accurate path does not yield
the correctly-rounded value for at least one rounding mode.
This list is sorted by increasing first values.
Warning: the values will be multiplied by 2^ie afterwards. */
static DB: [[f64; 3]; 21] = [
    [
        -24.96896403739838,
        1.9689130580912089,
        -1.3533717516041823e-32,
    ],
    [
        -7.421123488171717,
        1.2256796396039498,
        -2.8836527279112737e-32,
    ],
    [
        -0.25769183680034063,
        1.5456667084280928,
        1.1102230246251563e-16,
    ],
    [
        -0.11595781086814885,
        1.7810255849331564,
        -3.594938872459226e-33,
    ],
    [
        -0.1037516737020303,
        1.8028982463054148,
        1.1102230246251564e-16,
    ],
    [
        -0.019571676994419418,
        1.9612372097506556,
        1.1102230246251562e-16,
    ],
    [
        -0.0001997928992905134,
        1.9996004541159633,
        5.80132046194103e-34,
    ],
    [
        -8.542005785074093e-05,
        1.9998291671806772,
        -1.1102230246251564e-16,
    ],
    [
        -3.78359350122444e-09,
        0.9999999962164065,
        5.551115123125782e-17,
    ],
    [
        -1.8593351875450545e-09,
        0.9999999981406648,
        5.551115123125782e-17,
    ],
    [
        -1.4184426421272212e-09,
        0.9999999985815574,
        -5.551115123125782e-17,
    ],
    [
        -6.03594674424366e-10,
        0.9999999993964054,
        -5.551115123125782e-17,
    ],
    [
        1.1102230246251565e-16,
        1.0000000000000002,
        -1.1102230246251564e-16,
    ],
    [
        2.9103719434007728e-11,
        1.0000000000291036,
        1.1102230246251564e-16,
    ],
    [
        9.023326530003611e-11,
        1.0000000000902334,
        -1.1102230246251564e-16,
    ],
    [
        5.843446835809514e-10,
        1.0000000005843448,
        -1.1102230246251564e-16,
    ],
    [
        1.8367444215357543e-09,
        1.0000000018367443,
        1.1102230246251564e-16,
    ],
    [
        1.1033070105112662e-06,
        1.0000011033076193,
        -1.1102230246251564e-16,
    ],
    [
        1.809480788007787e-06,
        1.000001809482425,
        1.1102230246251563e-16,
    ],
    [
        0.005528277479060492,
        1.0055435866030302,
        1.1102230246251563e-16,
    ],
    [
        1.6744336219614115,
        1.3339430572217077,
        -1.1102230246251564e-16,
    ],
];

#[inline(never)]
fn as_exp_database(x: f64, f: f64) -> f64 {
    let mut a = 0usize;
    let mut b = DB.len();
    // binary search with invariant db[a][0] <= x < db[b][0]
    while a + 1 < b {
        let m = (a + b) / 2;
        if x < DB[m][0] {
            b = m;
        } else {
            a = m;
        }
    }
    // we have a+1=b thus db[a][0] <= x < db[a+1][0]
    if x == DB[a][0] {
        return DB[a][1] + DB[a][2];
    }
    f
}

// for 0 <= i < 2^6, t0[i] is a double-double approximation of 2^(i/2^6)
// some values are slightly modified with respect to the double-double
// rounding to nearest of 2^(i/2^6) to minimize the number of exceptional cases
// cf optimize() in exp.sage
#[allow(clippy::approx_constant)]
static T0: [[f64; 2]; 64] = [
    [0.0, 1.0],
    [-1.5234778603368574e-17, 1.0108892860517005],
    [5.1092250289734445e-17, 1.0218971486541166],
    [7.600838874027085e-18, 1.0330248790212284],
    [8.551889705537965e-17, 1.0442737824274138],
    [1.7593257387720928e-18, 1.0556451783605572],
    [-7.899853966841582e-17, 1.0671404006768237],
    [-6.656660436056593e-17, 1.0787607977571199],
    [-3.046782079812471e-17, 1.0905077326652577],
    [5.2660368715706944e-17, 1.102382583307841],
    [1.0410278456845571e-16, 1.1143867425958924],
    [5.1658567587954574e-17, 1.1265216186082418],
    [8.912812676025407e-17, 1.1387886347566916],
    [3.250710218863828e-17, 1.1511892299529827],
    [3.8292048369240935e-17, 1.1637248587775775],
    [5.554203254218078e-17, 1.1763969916502812],
    [3.982015231465646e-17, 1.189207115002721],
    [6.644981499252301e-17, 1.202156731452703],
    [-7.712630692681486e-17, 1.215247359980469],
    [-1.8987816313025275e-17, 1.22848053610687],
    [4.6580275918369356e-17, 1.241857812073484],
    [-6.7113898212968784e-18, 1.255380757024691],
    [2.667932131342186e-18, 1.2690509571917332],
    [1.713594918243561e-17, 1.2828700160787783],
    [2.5382502794888315e-17, 1.2968395546510096],
    [-7.181536135519454e-17, 1.3109612115247644],
    [-2.85873121003886e-17, 1.3252366431597413],
    [8.927282594831731e-17, 1.339667524053303],
    [7.700948379802988e-17, 1.3542555469368927],
    [9.593797919118849e-17, 1.3690024229745905],
    [-6.770511658794786e-17, 1.383909881963832],
    [-9.614213209051323e-17, 1.3989796725383112],
    [-9.667293313452913e-17, 1.4142135623730951],
    [-1.2031642489053654e-17, 1.42961333839197],
    [-3.023758134993988e-17, 1.4451808069770467],
    [-5.600377186075217e-17, 1.460917794180647],
    [-3.4839945568927964e-17, 1.4768261459394993],
    [1.4192920154284036e-17, 1.4929077282912648],
    [-1.016455327754295e-16, 1.5091644275934228],
    [-1.1024941712342561e-16, 1.5255981507445384],
    [7.949834809697623e-17, 1.5422108254079407],
    [3.7812070533575294e-17, 1.559004400237837],
    [-1.0136916471278304e-17, 1.5759808451078865],
    [-1.0094406542311962e-16, 1.593142151342267],
    [2.4707192569797888e-17, 1.6104903319492543],
    [-6.712955084707083e-17, 1.6280274218573478],
    [-1.0125679913674775e-16, 1.645755478153965],
    [5.890992696713101e-17, 1.6636765803267364],
    [8.199010020581498e-17, 1.681792830507429],
    [-8.0237193703977e-18, 1.7001063537185235],
    [-1.851380418263111e-17, 1.718619298122478],
    [3.164389299292957e-17, 1.7373338352737062],
    [2.960140695448874e-17, 1.7562521603732995],
    [6.429731796556572e-17, 1.7753764925265212],
    [1.822745842791208e-17, 1.7947090750031072],
    [-9.969531538920349e-17, 1.8142521755003989],
    [3.2831072242456266e-17, 1.8340080864093424],
    [9.761887490727595e-17, 1.8539791250833855],
    [-6.122763413004143e-17, 1.8741676341103],
    [3.40340353521653e-17, 1.8945759815869656],
    [-1.0619946056195964e-16, 1.9152065613971474],
    [1.0332385960676327e-16, 1.9360617934922943],
    [8.96076779103667e-17, 1.9571441241754002],
    [4.038875310927817e-17, 1.978456026387951],
];

// for 0 <= i < 2^6, t1[i] is a double-double approximation of 2^(i/2^12)
// some values are slightly modified with respect to the double-double
// rounding to nearest of 2^(i/2^6) to minimize the number of exceptional cases
// cf optimize() in exp.sage
static T1: [[f64; 2]; 64] = [
    [0.0, 1.0],
    [9.336185335478464e-17, 1.0001692397053021],
    [-5.141333931318957e-18, 1.0003385080526823],
    [6.962424022020571e-17, 1.0005078050469876],
    [-5.115123297685667e-17, 1.0006771306930664],
    [8.422990024586488e-17, 1.0008464849957674],
    [-2.8245220747761684e-17, 1.001015867959941],
    [-7.180424565592133e-17, 1.0011852795904375],
    [-1.8973728416792996e-17, 1.0013547198921082],
    [9.06044106726912e-17, 1.0015241888698057],
    [-7.173276349900319e-17, 1.0016936865283832],
    [-1.3307196246722662e-17, 1.0018632128726943],
    [2.5726925943221133e-17, 1.002032767907594],
    [-3.929937785484514e-17, 1.002202351637938],
    [8.461377247994717e-17, 1.0023719640685822],
    [-4.1948832416399384e-17, 1.0025416052043845],
    [-3.6366159286922646e-17, 1.0027112750502025],
    [-2.610944063243938e-17, 1.0028809736108952],
    [1.7530784779823324e-17, 1.0030507008913223],
    [5.753923525628267e-17, 1.0032204568963443],
    [-8.684922005117958e-18, 1.0033902416308227],
    [9.490035430981776e-17, 1.0035600550996193],
    [-8.710380605818421e-17, 1.0037298973075977],
    [3.4958916958571545e-17, 1.003899768259621],
    [9.753787549840241e-17, 1.004069667960554],
    [-1.0576221196292857e-16, 1.0042395964152628],
    [4.209188738127126e-17, 1.0044095536286128],
    [-1.6700166857554785e-17, 1.0045795396054717],
    [-1.6231463554124514e-17, 1.0047495543507072],
    [2.3028539278028117e-17, 1.004919597869188],
    [1.6418046976773032e-17, 1.0050896701657839],
    [3.726698431828413e-17, 1.005259771245365],
    [9.499186535455033e-17, 1.0054299011128027],
    [-8.68093131444458e-17, 1.0056000597729693],
    [4.0005474910301175e-17, 1.005770247230737],
    [7.190499111509974e-17, 1.00594046349098],
    [-1.3908068671065786e-17, 1.006110708558573],
    [-8.14020864257305e-17, 1.006280982438391],
    [-5.762151043749534e-17, 1.00645128513531],
    [6.745278477310456e-17, 1.0066216166542072],
    [1.8998557240346293e-17, 1.0067919769999607],
    [-9.637430032316406e-17, 1.006962366177449],
    [-1.2528654462453979e-17, 1.0071327841915512],
    [3.020578887843696e-17, 1.007303231047148],
    [-4.8693942586085655e-17, 1.0074737067491204],
    [5.224029937687454e-17, 1.0076442113023503],
    [-9.361543551478456e-17, 1.0078147447117207],
    [-8.65251323306195e-17, 1.007985306982115],
    [-3.252058756084309e-17, 1.0081558981184175],
    [-9.917232268060917e-17, 1.0083265181255139],
    [-7.136047404162522e-17, 1.0084971670082898],
    [-1.7268683712243214e-17, 1.0086678447716324],
    [-6.619954693673941e-17, 1.0088385514204294],
    [3.56545690151302e-17, 1.0090092869595693],
    [3.717310013708818e-17, 1.0091800513939415],
    [7.062572406825526e-17, 1.0093508447284363],
    [-1.432141230342882e-17, 1.0095216669679448],
    [1.5668188013134103e-17, 1.0096925181173586],
    [-1.1043695780393691e-16, 1.0098633981815708],
    [-5.767317427160399e-17, 1.0100343071654745],
    [4.835484978440381e-18, 1.0102052450739643],
    [7.015121289715443e-17, 1.0103762119119353],
    [7.161802873619573e-17, 1.0105472076842836],
    [1.050465913408405e-16, 1.010718232395906],
];

/* assume |x| > 2^-54 since the case |x| <= 0x1p-54 is treated directly
at the beginning of cr_exp() */
#[cold]
#[inline(never)]
fn as_exp_accurate(x: f64) -> f64 {
    // Q = ch[0]*x + ch[1]*x^2 + ... + ch[7]*x^7 approximates exp(x)-1 for |x| < 2^-13.528
    static CH: [[f64; 2]; 7] = [
        [1.0, 0.0],
        [0.5, 2.2752803697148587e-30],
        [0.16666666666666666, 9.251858538539695e-18],
        [0.041666666666666664, 2.311375602431149e-18],
        [0.008333333333333333, 1.1645814375470794e-19],
        [0.0013888888892440137, -7.022792706270539e-20],
        [0.0001984126983733902, -1.0602991841308864e-20],
    ];
    let mut ix = x.to_bits();
    let s = 5909.278887481194;
    let t = (x * s).round_ties_even();
    let jt = t as i64;
    let i0 = ((jt >> 6) & 0x3f) as usize;
    let i1 = (jt & 0x3f) as usize;
    let ie = jt >> 12;
    let (t0h, t0l) = (T0[i0][1], T0[i0][0]);
    let (t1h, t1l) = (T1[i1][1], T1[i1][0]);
    let mut tl = 0.0;
    let th = muldd(t0h, t0l, t1h, t1l, &mut tl);

    /* Use Cody-Waite argument reduction: since |x| < 745, we have |t| < 2^23,
    thus since l2h is exactly representable on 29 bits, l2h*t is exact. */
    let l2h = 0.00016922538588914904;
    let l2l = 1.0256140314162804e-14;
    let l2ll = 3.2042720746546034e-31;
    let dx = x - l2h * t;
    let mut dxl = l2l * t;
    let dxll = l2ll * t + l2l.mul_add(t, -dxl);
    let dxh = fasttwosum(dx, dxl, &mut dxl);
    dxl += dxll;
    let mut fl = 0.0;
    let mut fh = opolydd(dxh, dxl, &CH, &mut fl);
    fh = muldd(dxh, dxl, fh, fl, &mut fl);
    fh = muldd(fh, fl, th, tl, &mut fl);
    fh = fastsum(th, tl, fh, fl, &mut fl);
    if ix > 0xc086232bdd7abcd2 {
        // x < -0x1.6232bdd7abcd2p+9
        ix = ((1 - ie) as u64) << 52; // ix=2^-1022-ie with -1075 <= ie <= -1022
        let mut e = 0.0;
        fh = fasttwosum(f64::from_bits(ix), fh, &mut e);
        fl += e;
        fh = as_todenormal(fh + fl);
    } else {
        fh = fasttwosum(fh, fl, &mut fl);
        ix = fl.to_bits();
        let d = (ix.wrapping_add(4)) & (!0u64 >> 12); // d<=4 due to the single argument x=-0x1.07e05e6e03b83p-2
        if d <= 4 {
            fh = as_exp_database(x, fh);
        }
        fh = as_ldexp(fh, ie);
    }
    fh
}

pub(super) fn cr_exp(x: f64) -> f64 {
    let mut ix = x.to_bits();
    let aix = ix & (!0u64 >> 1);
    // exp(x) rounds to 1 to nearest for |x| <= 0x1p-54
    if aix <= 0x3c90000000000000 {
        // |x| <= 0x1p-54
        return 1.0 + x;
    }
    if aix >= 0x40862e42fefa39f0 {
        // |x| >= 0x1.62e42fefa39fp+9
        if aix > 0x7ff0000000000000 {
            return x + x; // nan
        }
        if aix == 0x7ff0000000000000 {
            // |x| = inf
            if (ix >> 63) != 0 {
                return 0.0; // x = -inf
            } else {
                return x; // x = inf
            }
        }
        if (ix >> 63) == 0 {
            // x >= 0x1.62e42fefa39fp+9
            let z = 8.98846567431158e+307;
            return z * z;
        }
        if aix >= 0x40874910d52d3052 {
            // x <= -0x1.74910d52d3052p+9
            let z = 2.2250738585072014e-308;
            return z * z;
        }
    }
    let s = 5909.278887481194;
    let t = (x * s).round_ties_even();
    let jt = t as i64;
    let i0 = ((jt >> 6) & 0x3f) as usize;
    let i1 = (jt & 0x3f) as usize;
    let ie = jt >> 12;
    let (t0h, t0l) = (T0[i0][1], T0[i0][0]);
    let (t1h, t1l) = (T1[i1][1], T1[i1][0]);
    let mut tl = 0.0;
    let th = muldd(t0h, t0l, t1h, t1l, &mut tl);
    let l2h = 0.00016922538588914904;
    let l2l = 1.0256140314162804e-14;
    /* Use Cody-Waite argument reduction: since |x| < 745, we have |t| < 2^23,
    thus since l2h is exactly representable on 29 bits, l2h*t is exact. */
    let dx = (x - l2h * t) + l2l * t;
    let dx2 = dx * dx;
    // |dx| < log(2)/2^13 (experimentally)
    /* 1 + x*(c0 + c1*x + ... + c3*x^3) is a degree-4 approximation of exp(x)
    on [-log(2)/2^13,log(2)/2^13] with absolute error bounded by 2^-76.173
    according to Sollya */
    static CH: [f64; 4] = [1.0, 0.5, 0.16666666674124284, 0.04166666665427057];
    let p = (CH[0] + dx * CH[1]) + dx2 * (CH[2] + dx * CH[3]);
    let mut fh = th;
    let tx = th * dx;
    let mut fl = tl + tx * p;
    let eps = 2.498747194400186e-19;
    if ix > 0xc086232bdd7abcd2 {
        // subnormal case: x < -0x1.6232bdd7abcd2p+9
        ix = ((1 - ie) as u64) << 52;
        let mut e = 0.0;
        fh = fasttwosum(f64::from_bits(ix), fh, &mut e);
        fl += e;
        let ub = fh + (fl + eps);
        let lb = fh + (fl - eps);
        if ub != lb {
            return as_exp_accurate(x);
        }
        fh = as_todenormal(lb);
    } else {
        let ub = fh + (fl + eps);
        let lb = fh + (fl - eps);
        if ub != lb {
            return as_exp_accurate(x);
        }
        fh = as_ldexp(lb, ie);
    }
    fh
}

#[cfg(test)]
mod tests {
    use super::cr_exp;

    fn check(name: &str, x: u64, want: u64) {
        let got = cr_exp(f64::from_bits(x)).to_bits();
        assert_eq!(
            got, want,
            "{name}: cr_exp({x:016x}) = {got:016x}, want {want:016x}"
        );
    }

    #[test]
    fn special_values() {
        assert!(cr_exp(f64::NAN).is_nan(), "NaN");
        check("+0", 0x0000000000000000, 0x3ff0000000000000);
        check("-0", 0x8000000000000000, 0x3ff0000000000000);
        check("+inf", 0x7ff0000000000000, 0x7ff0000000000000);
        check("-inf", 0xfff0000000000000, 0x0000000000000000);
        check("min subnormal", 0x0000000000000001, 0x3ff0000000000000);
        check("-min subnormal", 0x8000000000000001, 0x3ff0000000000000);
    }

    #[test]
    fn thresholds() {
        check(
            "2^-54 (returns 1 + x)",
            0x3c90000000000000,
            0x3ff0000000000000,
        );
        check("2^-53 (main path)", 0x3ca0000000000000, 0x3ff0000000000001);
        check(
            "largest finite result",
            0x40862e42fefa39ef,
            0x7fefffffffffff2a,
        );
        check("overflow threshold", 0x40862e42fefa39f0, 0x7ff0000000000000);
        check("1000", 0x408f400000000000, 0x7ff0000000000000);
        check(
            "just above subnormal switch",
            0x4086232bdd7abcd3,
            0x7fd0000000000184,
        );
        check(
            "first subnormal result",
            0xc086232bdd7abcd3,
            0x000ffffffffffe7c,
        );
        check("subnormal -708.5", 0xc086240000000000, 0x000e6cf6d08897ac);
        check("subnormal -709.5", 0xc0862c0000000000, 0x00054e90c99fb878);
        check(
            "min subnormal result",
            0xc0874910d52d3051,
            0x0000000000000001,
        );
        check(
            "underflow threshold",
            0xc0874910d52d3052,
            0x0000000000000000,
        );
        check("-1000", 0xc08f400000000000, 0x0000000000000000);
    }

    #[test]
    fn fast_and_accurate_paths() {
        check("1", 0x3ff0000000000000, 0x4005bf0a8b145769);
        check("-1", 0xbff0000000000000, 0x3fd78b56362cef38);
        check("10", 0x4024000000000000, 0x40d5829dcf950560);
        check("-10", 0xc024000000000000, 0x3f07cd79b5647c9b);
        check("0.5", 0x3fe0000000000000, 0x3ffa61298e1e069c);
        check(
            "accurate 17.519764420075667",
            0x4031850f47f1d972,
            0x41835e7680b63666,
        );
        check(
            "accurate -25.79963697669376",
            0xc039ccb50247926f,
            0x3d9b74743bcd29d3,
        );
    }

    #[test]
    fn database_entries() {
        check(
            "db -24.96896403739838",
            0xc038f80e06f3a04c,
            0x3daf80aafa92b498,
        );
        check(
            "db -0.25769183680034063",
            0xbfd07e05e6e03b83,
            0x3fe8bb0d03b36f1c,
        );
        check(
            "db 0.005528277479060492",
            0x3f76a4d1af9cc989,
            0x3ff016b4df3299d7,
        );
        check(
            "db 1.6744336219614115",
            0x3ffaca7ae8da5a7b,
            0x401557d4acd7e557,
        );
    }
}
