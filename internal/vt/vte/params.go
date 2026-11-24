// Transliterated from vte 0.15.0 src/params.rs (Apache-2.0 OR MIT, used under MIT: Copyright (c) 2016 Joe Wilm).

package vte

const MaxParams = 32

// Params is a fixed-size parameter list with optional subparameters.
type Params struct {
	subparams        [MaxParams]uint8
	params           [MaxParams]uint16
	currentSubparams uint8
	len              int
}

func (p *Params) Len() int { return p.len }

func (p *Params) IsEmpty() bool { return p.len == 0 }

func (p *Params) isFull() bool { return p.len == MaxParams }

func (p *Params) clear() {
	p.currentSubparams = 0
	p.len = 0
}

func (p *Params) push(item uint16) {
	p.subparams[p.len-int(p.currentSubparams)] = p.currentSubparams + 1
	p.params[p.len] = item
	p.currentSubparams = 0
	p.len++
}

func (p *Params) extend(item uint16) {
	p.subparams[p.len-int(p.currentSubparams)] = p.currentSubparams + 1
	p.params[p.len] = item
	p.currentSubparams++
	p.len++
}

// Iter returns an iterator over parameters; each item holds a parameter and its subparameters.
func (p *Params) Iter() ParamsIter { return ParamsIter{params: p} }

type ParamsIter struct {
	params *Params
	index  int
}

func (it *ParamsIter) Next() ([]uint16, bool) {
	if it.index >= it.params.len {
		return nil, false
	}
	n := int(it.params.subparams[it.index])
	param := it.params.params[it.index : it.index+n]
	it.index += n
	return param, true
}

// Collect returns every parameter as a slice of subparameter slices.
func (p *Params) Collect() [][]uint16 {
	var out [][]uint16
	it := p.Iter()
	for {
		param, ok := it.Next()
		if !ok {
			return out
		}
		out = append(out, param)
	}
}
