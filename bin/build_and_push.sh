#!/bin/sh

# Parse command line arguments
while [[ "$#" -gt 0 ]]; do
    case $1 in
        --dockerfile) DOCKERFILE="$2"; shift ;;
        --ecr-repo) ECR_REPO="$2"; shift ;;
        --region) REGION="$2"; shift ;;
        *) echo "Unknown parameter: $1"; exit 1 ;;
    esac
    shift
done

REGION=${REGION:-us-east-2}
ECR_REGISTRY=377928551571.dkr.ecr.$REGION.amazonaws.com

# Check if required arguments are provided
if [ -z "$DOCKERFILE" ] || [ -z "$ECR_REPO" ]; then
    echo "Usage: $0 --dockerfile <path_to_dockerfile> --ecr-repo <ecr_repository> [--environment <environment>] [--region <aws_region>]"
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

TAG_1=$ECR_URL\:$COMMIT_HASH
TAG_2=$ECR_URL\:latest

# Tag and push the image with latest and commit hash
docker tag $IMAGE_NAME:latest $TAG_1
docker tag $IMAGE_NAME:latest $TAG_2
docker push $TAG_1
docker push $TAG_2