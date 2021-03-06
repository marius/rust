// Copyright 2012-2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use rustc::mir::interpret::ErrorHandled;
use rustc_mir::const_eval::const_field;
use rustc::mir;
use rustc_data_structures::indexed_vec::Idx;
use rustc::mir::interpret::{GlobalId, ConstValue};
use rustc::ty::{self, Ty};
use rustc::ty::layout::{self, LayoutOf};
use syntax::source_map::Span;
use traits::*;

use super::FunctionCx;

impl<'a, 'tcx: 'a, Bx: BuilderMethods<'a, 'tcx>> FunctionCx<'a, 'tcx, Bx> {
    fn fully_evaluate(
        &mut self,
        bx: &Bx,
        constant: &'tcx ty::Const<'tcx>,
    ) -> Result<&'tcx ty::Const<'tcx>, ErrorHandled> {
        match constant.val {
            ConstValue::Unevaluated(def_id, ref substs) => {
                let tcx = bx.tcx();
                let param_env = ty::ParamEnv::reveal_all();
                let instance = ty::Instance::resolve(tcx, param_env, def_id, substs).unwrap();
                let cid = GlobalId {
                    instance,
                    promoted: None,
                };
                tcx.const_eval(param_env.and(cid))
            },
            _ => Ok(constant),
        }
    }

    pub fn eval_mir_constant(
        &mut self,
        bx: &Bx,
        constant: &mir::Constant<'tcx>,
    ) -> Result<&'tcx ty::Const<'tcx>, ErrorHandled> {
        let c = self.monomorphize(&constant.literal);
        self.fully_evaluate(bx, c)
    }

    /// process constant containing SIMD shuffle indices
    pub fn simd_shuffle_indices(
        &mut self,
        bx: &Bx,
        span: Span,
        ty: Ty<'tcx>,
        constant: Result<&'tcx ty::Const<'tcx>, ErrorHandled>,
    ) -> (Bx::Value, Ty<'tcx>) {
        constant
            .and_then(|c| {
                let field_ty = c.ty.builtin_index().unwrap();
                let fields = match c.ty.sty {
                    ty::Array(_, n) => n.unwrap_usize(bx.tcx()),
                    ref other => bug!("invalid simd shuffle type: {}", other),
                };
                let values: Result<Vec<_>, ErrorHandled> = (0..fields).map(|field| {
                    let field = const_field(
                        bx.tcx(),
                        ty::ParamEnv::reveal_all(),
                        self.instance,
                        None,
                        mir::Field::new(field as usize),
                        c,
                    )?;
                    if let Some(prim) = field.val.try_to_scalar() {
                        let layout = bx.cx().layout_of(field_ty);
                        let scalar = match layout.abi {
                            layout::Abi::Scalar(ref x) => x,
                            _ => bug!("from_const: invalid ByVal layout: {:#?}", layout)
                        };
                        Ok(bx.cx().scalar_to_backend(
                            prim, scalar,
                            bx.cx().immediate_backend_type(layout),
                        ))
                    } else {
                        bug!("simd shuffle field {:?}", field)
                    }
                }).collect();
                let llval = bx.cx().const_struct(&values?, false);
                Ok((llval, c.ty))
            })
            .unwrap_or_else(|_| {
                bx.tcx().sess.span_err(
                    span,
                    "could not evaluate shuffle_indices at compile time",
                );
                // We've errored, so we don't have to produce working code.
                let ty = self.monomorphize(&ty);
                let llty = bx.cx().backend_type(bx.cx().layout_of(ty));
                (bx.cx().const_undef(llty), ty)
            })
    }
}
