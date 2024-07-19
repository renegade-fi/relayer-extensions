#!/bin/sh
REGION=us-east-2
ENVIRONMENT=testnet
ECR_REGISTRY=377928551571.dkr.ecr.us-east-2.amazonaws.com

# Parse command line arguments
while [[ "$#" -gt 0 ]]; do
    case $1 in
        --dockerfile) DOCKERFILE="$2"; shift ;;
        --ecr-repo) ECR_REPO="$2"; shift ;;
        --environment) ENVIRONMENT="$2"; shift ;;
        *) echo "Unknown parameter: $1"; exit 1 ;;
    esac
    shift
done

# Check if required arguments are provided
if [ -z "$DOCKERFILE" ] || [ -z "$ECR_REPO" ]; then
    echo "Usage: $0 --dockerfile <path_to_dockerfile> --ecr-repo <ecr_repository> [--environment <environment>]"
    exit 1
fi

ECR_URL="$ECR_REGISTRY/$ECR_REPO"
IMAGE_NAME=$ECR_REPO

# Get the current commit hash
COMMIT_HASH=$(git rev-parse --short HEAD)

# Build the Docker image
docker build -t $IMAGE_NAME:latest -f "$DOCKERFILE" .

# Login to ECR
aws ecr get-login-password --region $REGION | docker login --username AWS --password-stdin $ECR_REGISTRY

# Tag and push the image with latest and commit hash
docker tag $IMAGE_NAME:latest $ECR_URL:latest
docker tag $IMAGE_NAME:latest $ECR_URL:$COMMIT_HASH
docker push $ECR_URL:latest
docker push $ECR_URL:$COMMIT_HASH