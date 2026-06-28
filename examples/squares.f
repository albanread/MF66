\ squares.f — squares 1..10
: sq  dup * ;
: table  11 1 do  i . ." -> " i sq . cr  loop ;
table
