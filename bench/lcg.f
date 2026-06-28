\ 64-bit LCG, 100,000,000 iterations (non-foldable recurrence). Prints signed final state.
: lcg ( -- x ) 1  100000000 0 do  6364136223846793005 * 1442695040888963407 +  loop ;
utime drop  lcg  utime drop
rot -  ." RESULT " swap . cr  ." MICROS " . cr
