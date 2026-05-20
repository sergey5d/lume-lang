# SKIP
#
# Candidate enum-based Either design with behavior defined on the enclosing
# enum instead of inside each case body.

enum Either[L, R] {

   case Left {
        value L
    }

    case Right {
        value R
    }

    def isLeft() Bool = match this {
        case Left(_) => true
        case Right(_) => false
    }

    def map[T](f R -> T) Either[L, T] = match this {
        case Left(value) => Left(value)
        case Right(value) => Right(f(value))
    }
}
