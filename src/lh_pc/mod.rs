use crate::lh_pc::error::LHPCError;
use crate::lh_pc::linear_hash::data_structures::LHUniversalParameters;
use crate::lh_pc::linear_hash::LinearHashFunction;
use crate::{BTreeMap, BTreeSet, ToString, Vec, PolynomialLabel};
use crate::{
    Error, Evaluations, LabeledCommitment, LabeledPolynomial, PCCommitterKey, PCVerifierKey,
    Polynomial, PolynomialCommitment, QuerySet,
};
use ark_ff::{Field, UniformRand, Zero};
use ark_std::vec;
use core::marker::PhantomData;
use rand_core::RngCore;

pub mod data_structures;
use ark_poly::UVPolynomial;
pub use data_structures::*;

pub mod error;
pub mod linear_hash;

pub struct LinearHashPC<F: Field, P: UVPolynomial<F>, LH: LinearHashFunction<F>> {
    _field: PhantomData<F>,
    _polynomial: PhantomData<P>,
    _lh: PhantomData<LH>,
}

impl<F: Field, P: UVPolynomial<F>, LH: LinearHashFunction<F> + 'static> LinearHashPC<F, P, LH> {
    fn check_degrees(supported_degree: usize, p: &P) -> Result<(), LHPCError> {
        if p.degree() < 1 {
            return Err(LHPCError::pc_error(Error::DegreeIsZero));
        } else if p.degree() > supported_degree {
            return Err(LHPCError::pc_error(Error::TooManyCoefficients {
                num_coefficients: p.degree() + 1,
                num_powers: supported_degree + 1,
            }));
        }

        Ok(())
    }

    fn check_degrees_and_bounds(
        supported_degree: usize,
        p: &LabeledPolynomial<F, P>,
    ) -> Result<(), LHPCError> {
        Self::check_degrees(supported_degree, p.polynomial())?;
        if p.degree_bound().is_some() {
            //TODO: Error
        }

        Ok(())
    }
}

impl<F: Field, P: UVPolynomial<F>, LH: LinearHashFunction<F> + 'static> PolynomialCommitment<F, P>
    for LinearHashPC<F, P, LH>
{
    type UniversalParams = UniversalParameters<F, LH>;
    type CommitterKey = CommitterKey<F, LH>;
    type VerifierKey = VerifierKey<F, LH>;
    type PreparedVerifierKey = VerifierKey<F, LH>;
    type Commitment = Commitment<F, LH>;
    type PreparedCommitment = Commitment<F, LH>;
    type Randomness = Randomness;
    type Proof = Proof<F, P>;
    type BatchProof = Vec<Proof<F, P>>;
    type Error = LHPCError;

    fn setup<R: RngCore>(
        max_degree: usize,
        _: Option<usize>,
        rng: &mut R,
    ) -> Result<Self::UniversalParams, Self::Error> {
        let lh_pp = LH::setup(max_degree + 1, rng).map_err(|e| LHPCError::lh_error(e))?;
        Ok(UniversalParameters(lh_pp))
    }

    fn trim(
        pp: &Self::UniversalParams,
        supported_degree: usize,
        _supported_hiding_bound: usize,
        _enforced_degree_bounds: Option<&[usize]>,
    ) -> Result<(Self::CommitterKey, Self::VerifierKey), Self::Error> {
        if supported_degree + 1 > pp.0.max_elems_len() {
            return Err(LHPCError::pc_error(Error::TrimmingDegreeTooLarge));
        }

        let lh_ck = LH::trim(&pp.0, supported_degree + 1).map_err(|e| LHPCError::lh_error(e))?;
        let ck = CommitterKey(lh_ck.clone());
        let vk = VerifierKey(lh_ck);
        Ok((ck, vk))
    }

    fn commit<'a>(
        ck: &Self::CommitterKey,
        polynomials: impl IntoIterator<Item = &'a LabeledPolynomial<F, P>>,
        _rng: Option<&mut dyn RngCore>,
    ) -> Result<
        (
            Vec<LabeledCommitment<Self::Commitment>>,
            Vec<Self::Randomness>,
        ),
        Self::Error,
    >
    where
        P: 'a,
    {
        let supported_degree = ck.supported_degree();
        let mut commitments = Vec::new();
        for labeled_polynomial in polynomials {
            Self::check_degrees_and_bounds(supported_degree, labeled_polynomial)?;

            let polynomial = labeled_polynomial.polynomial();
            let mut coeffs = polynomial.coeffs().to_vec();
            while coeffs.len() < ck.supported_degree() + 1 {
                coeffs.push(F::zero());
            }

            let lh_commitment =
                LH::commit(&ck.0, coeffs.as_slice()).map_err(|e| LHPCError::lh_error(e))?;
            let comm = Commitment(lh_commitment);
            let labeled_comm = LabeledCommitment::new(
                labeled_polynomial.label().clone(),
                comm,
                None,
            );
            commitments.push(labeled_comm);
        }

        let randomness = vec![Randomness(()); commitments.len()];
        Ok((commitments, randomness))
    }

    fn open_individual_opening_challenges<'a>(
        ck: &Self::CommitterKey,
        labeled_polynomials: impl IntoIterator<Item = &'a LabeledPolynomial<F, P>>,
        _commitments: impl IntoIterator<Item = &'a LabeledCommitment<Self::Commitment>>,
        _point: &F,
        opening_challenges: &dyn Fn(u64) -> F,
        _rands: impl IntoIterator<Item = &'a Self::Randomness>,
        _rng: Option<&mut dyn RngCore>,
    ) -> Result<Self::Proof, Self::Error>
    where
        Self::Randomness: 'a,
        Self::Commitment: 'a,
        P: 'a,
    {
        let supported_degree = ck.supported_degree();
        let mut combined_polynomial = P::zero();

        let mut i = 0;
        for labeled_polynomial in labeled_polynomials {
            Self::check_degrees_and_bounds(supported_degree, labeled_polynomial)?;
            combined_polynomial += (opening_challenges(i), labeled_polynomial.polynomial());
            i += 1;
        }

        let combined_polynomial = LabeledPolynomial::new(PolynomialLabel::new(), combined_polynomial, None, None);

        let proof = Proof(combined_polynomial);
        Ok(proof)
    }

    fn check_individual_opening_challenges<'a>(
        vk: &Self::VerifierKey,
        commitments: impl IntoIterator<Item = &'a LabeledCommitment<Self::Commitment>>,
        point: &F,
        values: impl IntoIterator<Item = F>,
        proof: &Self::Proof,
        opening_challenges: &dyn Fn(u64) -> F,
        _rng: Option<&mut dyn RngCore>,
    ) -> Result<bool, Self::Error>
    where
        Self::Commitment: 'a,
    {
        let supported_degree = vk.supported_degree();
        let check = Self::check_degrees(supported_degree, &proof.0);
        if check.is_err() {
            return Ok(false);
        }

        let mut accumulated_value = F::zero();
        let mut scalar_commitment_pairs = Vec::new();

        let mut i = 0;
        for (commitment, value) in commitments.into_iter().zip(values) {
            if commitment.degree_bound().is_some() {
                return Ok(false);
            }

            let cur_challenge = opening_challenges(i);
            accumulated_value += &(value.mul(cur_challenge));
            scalar_commitment_pairs.push((cur_challenge, &commitment.commitment().0));
            i += 1;
        }

        let expected_value = proof.0.evaluate(point);
        if accumulated_value != expected_value {
            return Ok(false);
        }

        let mut coeffs = proof.0.coeffs().to_vec();
        while coeffs.len() < supported_degree + 1 {
            coeffs.push(F::zero());
        }

        let accumulated_commitment = scalar_commitment_pairs.into_iter().sum();
        let expected_commitment =
            LH::commit(&vk.0, coeffs.as_slice()).map_err(|e| LHPCError::lh_error(e))?;

        Ok(expected_commitment.eq(&accumulated_commitment))
    }

    fn batch_check_individual_opening_challenges<'a, R: RngCore>(
        vk: &Self::VerifierKey,
        commitments: impl IntoIterator<Item = &'a LabeledCommitment<Self::Commitment>>,
        query_set: &QuerySet<F>,
        values: &Evaluations<F, P::Point>,
        proof: &Self::BatchProof,
        opening_challenges: &dyn Fn(u64) -> F,
        rng: &mut R,
    ) -> Result<bool, Self::Error>
    where
        Self::Commitment: 'a,
    {
        let supported_degree = vk.supported_degree();
        let commitments: BTreeMap<_, _> = commitments.into_iter().map(|c| (c.label(), c)).collect();
        let mut query_to_labels_map = BTreeMap::new();

        for (label, point) in query_set.iter() {
            let labels = query_to_labels_map.entry(point).or_insert(BTreeSet::new());
            labels.insert(label);
        }

        assert_eq!(proof.len(), query_to_labels_map.len());

        let mut expected_value: F = F::zero();
        let mut randomizers = Vec::new();
        let mut proof_commitments = Vec::new();

        let mut accumulated_value = F::zero();
        let mut scalar_commitment_pairs = Vec::new();

        for ((query, labels), p) in query_to_labels_map.into_iter().zip(proof) {
            let query_challenge: F = u128::rand(rng).into();
            expected_value += &p.0.evaluate(&query.1).mul(query_challenge);

            let mut coeffs = p.0.coeffs().to_vec();
            while coeffs.len() < supported_degree + 1 {
                coeffs.push(F::zero());
            }
            let proof_commitment =
                LH::commit(&vk.0, coeffs.as_slice()).map_err(|e| LHPCError::lh_error(e))?;
            randomizers.push(query_challenge);
            proof_commitments.push(proof_commitment);

            let mut i = 0;
            for label in labels.into_iter() {
                let cur_challenge = query_challenge.mul(opening_challenges(i));
                let commitment = commitments.get(label).ok_or(Error::MissingPolynomial {
                    label: label.to_string(),
                })?;

                let v_i =
                    values
                        .get(&(label.clone(), query.1))
                        .ok_or(Error::MissingEvaluation {
                            label: label.to_string(),
                        })?;

                accumulated_value += &(v_i.mul(cur_challenge));
                scalar_commitment_pairs.push((cur_challenge, &commitment.commitment().0));
                i += 1;
            }
        }

        if expected_value != accumulated_value {
            return Ok(false);
        }

        let expected_scalar_commitment_pairs: Vec<(F, &LH::Commitment)> =
            randomizers.into_iter().zip(&proof_commitments).collect();

        let accumulated_commitment: LH::Commitment = scalar_commitment_pairs.into_iter().sum();
        let expected_commitment: LH::Commitment =
            expected_scalar_commitment_pairs.into_iter().sum();

        Ok(accumulated_commitment == expected_commitment)
    }

    /*
    fn open_combinations<'a>(
        ck: &Self::CommitterKey,
        lc_s: impl IntoIterator<Item = &'a LinearCombination<F>>,
        polynomials: impl IntoIterator<Item = &'a LabeledPolynomial<'a, F>>,
        commitments: impl IntoIterator<Item = &'a LabeledCommitment<Self::Commitment>>,
        query_set: &QuerySet<F>,
        opening_challenge: F,
        rands: impl IntoIterator<Item = &'a Self::Randomness>,
        rng: Option<&mut dyn RngCore>,
    ) -> Result<BatchLCProof<F, Self>, Self::Error>
    where
        Self::Randomness: 'a,
        Self::Commitment: 'a,
    {
        let label_map = polynomials
            .into_iter()
            .zip(commitments)
            .map(|(p, c)| (p.label(), (p, c)))
            .collect::<BTreeMap<_, _>>();

        let mut lc_polynomials = Vec::new();
        let mut lc_commitments = Vec::new();
        let mut lc_info = Vec::new();

        for lc in lc_s {
            let lc_label = lc.label().clone();
            let mut poly = Polynomial::zero();
            let mut scalar_commitment_pairs = Vec::new();

            let num_polys = lc.len();
            for (coeff, label) in lc.iter().filter(|(_, l)| !l.is_one()) {
                let label: &String = label.try_into().expect("cannot be one!");
                let &(cur_poly, cur_comm) =
                    label_map.get(label).ok_or(Error::MissingPolynomial {
                        label: label.to_string(),
                    })?;

                if num_polys == 1 && cur_poly.degree_bound().is_some() {
                    assert!(
                        coeff.is_one(),
                        "Coefficient must be one for degree-bounded equations"
                    );
                    degree_bound = cur_poly.degree_bound();
                } else if cur_poly.degree_bound().is_some() {
                    eprintln!("Degree bound when number of equations is non-zero");
                    return Err(Self::Error::EquationHasDegreeBounds(lc_label));
                }

                poly += (*coeff, cur_poly.polynomial());
                scalar_commitment_pairs.push((*coeff, cur_comm.commitment()));
            }

            let lc_poly =
                LabeledPolynomial::new_owned(lc_label.clone(), poly, degree_bound, hiding_bound);
            lc_polynomials.push(lc_poly);
            lc_randomness.push(randomness);
            lc_commitments.push(Self::combine_commitments(coeffs_and_comms));
            lc_info.push((lc_label, degree_bound));
        }

        let comms = Self::normalize_commitments(lc_commitments);
        let lc_commitments = lc_info
            .into_iter()
            .zip(comms)
            .map(|((label, d), c)| LabeledCommitment::new(label, c, d))
            .collect::<Vec<_>>();

        let proof = Self::batch_open(
            ck,
            lc_polynomials.iter(),
            lc_commitments.iter(),
            &query_set,
            opening_challenge,
            lc_randomness.iter(),
            rng,
        )?;

        Ok(BatchLCProof { proof, evals: None })
    }

     */
}

#[cfg(test)]
mod tests {
    #![allow(non_camel_case_types)]

    use super::linear_hash::pedersen::PedersenCommitment;
    use super::LinearHashPC;

    use ark_ed_on_bls12_381::EdwardsAffine;
    use ark_ed_on_bls12_381::Fr;
    use ark_ff::PrimeField;
    use blake2::Blake2s;
    use ark_poly::{univariate::DensePolynomial, UVPolynomial};

    type PC<F, P, LH> = LinearHashPC<F, P, LH>;
    type LH_PED = PedersenCommitment<EdwardsAffine, Blake2s>;
    type PC_PED = PC<Fr, DensePolynomial<Fr>, LH_PED>;

    fn rand_poly<F: PrimeField>(
        degree: usize,
        _: Option<usize>,
        rng: &mut rand::prelude::StdRng,
    ) -> DensePolynomial<F> {
        DensePolynomial::rand(degree, rng)
    }

    fn rand_point<F: PrimeField>(_: Option<usize>, rng: &mut rand::prelude::StdRng) -> F {
        F::rand(rng)
    }

    #[test]
    fn single_poly_test() {
        use crate::tests::*;
        single_poly_test::<_, _, PC_PED>(None, rand_poly::<Fr>, rand_point::<Fr>)
            .expect("test failed for pedersen commitment");
    }

    #[test]
    fn quadratic_poly_degree_bound_multiple_queries_test() {
        use crate::tests::*;
        quadratic_poly_degree_bound_multiple_queries_test::<_, _, PC_PED>(
            rand_poly::<Fr>,
            rand_point::<Fr>,
        )
        .expect("test failed for pedersen commitment");
    }

    #[test]
    fn linear_poly_degree_bound_test() {
        use crate::tests::*;
        linear_poly_degree_bound_test::<_, _, PC_PED>(rand_poly::<Fr>, rand_point::<Fr>)
            .expect("test failed for pedersen commitment");
    }

    #[test]
    fn single_poly_degree_bound_test() {
        use crate::tests::*;
        single_poly_degree_bound_test::<_, _, PC_PED>(rand_poly::<Fr>, rand_point::<Fr>)
            .expect("test failed for pedersen commitment");
    }

    #[test]
    fn single_poly_degree_bound_multiple_queries_test() {
        use crate::tests::*;
        single_poly_degree_bound_multiple_queries_test::<_, _, PC_PED>(
            rand_poly::<Fr>,
            rand_point::<Fr>,
        )
        .expect("test failed for pedersen commitment");
    }

    #[test]
    fn two_polys_degree_bound_single_query_test() {
        use crate::tests::*;
        two_polys_degree_bound_single_query_test::<_, _, PC_PED>(rand_poly::<Fr>, rand_point::<Fr>)
            .expect("test failed for pedersen commitment");
    }

    #[test]
    fn full_end_to_end_test() {
        use crate::tests::*;
        full_end_to_end_test::<_, _, PC_PED>(None, rand_poly::<Fr>, rand_point::<Fr>)
            .expect("test failed for pedersen commitment");
    }

    #[test]
    fn single_equation_test() {
        use crate::tests::*;
        single_equation_test::<_, _, PC_PED>(None, rand_poly::<Fr>, rand_point::<Fr>)
            .expect("test failed for pedersen commitment");
    }

    #[test]
    fn two_equation_test() {
        use crate::tests::*;
        two_equation_test::<_, _, PC_PED>(None, rand_poly::<Fr>, rand_point::<Fr>)
            .expect("test failed for pedersen commitment");
    }

    #[test]
    fn two_equation_degree_bound_test() {
        use crate::tests::*;
        two_equation_degree_bound_test::<_, _, PC_PED>(rand_poly::<Fr>, rand_point::<Fr>)
            .expect("test failed for pedersen commitment");
    }

    #[test]
    fn full_end_to_end_equation_test() {
        use crate::tests::*;
        full_end_to_end_equation_test::<_, _, PC_PED>(None, rand_poly::<Fr>, rand_point::<Fr>)
            .expect("test failed for pedersen commitment");
    }

    #[test]
    #[should_panic]
    fn bad_degree_bound_test() {
        use crate::tests::*;
        bad_degree_bound_test::<_, _, PC_PED>(rand_poly::<Fr>, rand_point::<Fr>)
            .expect("test failed for pedersen commitment");
    }
}