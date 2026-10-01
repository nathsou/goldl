# Component pattern sources

The stable circuitry in this directory comes from the pattern files accompanying
*Conway's Game of Life: Mathematics and Construction* by Nathaniel Johnston and Dave Greene
(https://github.com/nathanieljohnston/game-of-life-book), licensed CC BY 4.0.
Original discoverers are credited in each file's `#O`/`#C` header lines:

* `snark.rle`: Snark, Mike Playle (2013)
* `duplicator.rle`, `tripler.rle`: syringe-based glider duplicator / tripler
* `syringe.rle`: Syringe, Tanner Jacobi (2015)
* `rectifier.rle`: Rectifier, Adam P. Goucher (2009)
* `transparent_lane.rle`: Herschel conduit with a transparent lane
* `demultiplexer.rle`: Demultiplexer, Brice Due (2006)
* `bandersnatch.rle`: Bandersnatch + Snark colour-changing reflector
* `rephaser_*.rle`: Ekström's stable glider phase changers (delay k mod 8, colour c)

GoLDL re-verifies every component with its own Life engine (`cargo test -p goldl`);
nothing is used on trust.
