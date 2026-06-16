fn main() {
    let old = [1.0, 0.0, 0.0, 1.0, 50.0, 100.0];
    let m = [2.0, 0.0, 0.0, 3.0, 10.0, 20.0];
    
    let a = m[0]; let b = m[1]; let c = m[2]; let d = m[3]; let e = m[4]; let f = m[5];
    
    // My interpreter.rs implementation
    let mut new_ctm = [0.0; 6];
    new_ctm[0] = old[0] * a + old[2] * b;
    new_ctm[1] = old[1] * a + old[3] * b;
    new_ctm[2] = old[0] * c + old[2] * d;
    new_ctm[3] = old[1] * c + old[3] * d;
    new_ctm[4] = old[0] * e + old[2] * f + old[4];
    new_ctm[5] = old[1] * e + old[3] * f + old[5];
    println!("My interpreter.rs math: {:?}", new_ctm);

    // Standard CTM * M
    let mut new_ctm2 = [0.0; 6];
    new_ctm2[0] = old[0] * a + old[1] * c;
    new_ctm2[1] = old[0] * b + old[1] * d;
    new_ctm2[2] = old[2] * a + old[3] * c;
    new_ctm2[3] = old[2] * b + old[3] * d;
    new_ctm2[4] = old[4] * a + old[5] * c + e;
    new_ctm2[5] = old[4] * b + old[5] * d + f;
    println!("Standard CTM * M: {:?}", new_ctm2);
}
