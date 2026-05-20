# SKIP
#
# Candidate enum-based Either design preserved from stdlib experimentation.
# It is intentionally skipped because the current enum grammar/runtime does not
# yet support this shape.

enum Either[L, R] {

    def isLeft() Bool

    def map[T](f R -> T) Either[L, T]

    case Left {

        value L

        def isLeft() Bool = true

        def map[T](f R -> T) Either[L, T] = this
    }

    case Right {

        value R

        def isLeft() Bool = false

        def map[T](f R -> T) Either[L, T] = Right(f(val))
    }
}

interface EitherLike {
    def isLeft() Bool = false

    def map[T](f R -> T) Either[L, T] = Right(f(val))
}


enum Either[L, R] {

    case Left {
        value L
    }

    case Right {
        value R
    }
}

impl Either[L,R] {
    def isLeft() Bool = this match {
        
    }
}

impl Either[L,R].Left {
    
    def isLeft() Bool = false

    def map[T](f R -> T) Either[L, T] = Right(f(val))
}
