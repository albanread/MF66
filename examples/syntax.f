\ syntax.f — Forth highlighting showcase
( paren comment — stack effect:  n -- n*n )
: sq   dup * ;
: greet   ." Hello, MF66 Forth!" cr ;

: classify ( n -- )
   dup 0< if    ." negative"
   else dup 0= if ." zero" else ." positive" then
   then  drop ;

: countdown ( n -- )
   0 ?do  i .  loop ;

3.14159e   fconstant pi
42         constant answer
variable   counter
$ff        constant all-bits
