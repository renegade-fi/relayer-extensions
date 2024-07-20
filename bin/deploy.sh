#!/bin/sh
set -e

DEFAULT_REGION=us-east-2
DEFAULT_IMAGE_TAG=$(git rev-parse --short HEAD)

# Parse command line arguments
while [[ "$#" -gt 0 ]]; do
    case $1 in
        --environment) ENVIRONMENT="$2"; shift ;;
        --resource) RESOURCE="$2"; shift ;;
        --region) REGION="$2"; shift ;;
        --image-tag) IMAGE_TAG="$2"; shift ;;
        *) echo "Unknown parameter: $1"; exit 1 ;;
    esac
    shift
done

# Set defaults if not provided
REGION=${REGION:-$DEFAULT_REGION}
IMAGE_TAG=${IMAGE_TAG:-$DEFAULT_IMAGE_TAG}

# Check if required arguments are provided
if [ -z "$ENVIRONMENT" ] || [ -z "$RESOURCE" ]; then
    echo "Usage: $0 --environment <env> --resource <resource> [--region <region>] [--image-tag <tag>]"
    exit 1
fi

ECR_REGISTRY=377928551571.dkr.ecr.$REGION.amazonaws.com

# Derive values from environment and resource
CLUSTER_NAME="$ENVIRONMENT-$RESOURCE-cluster"
SERVICE_NAME="$ENVIRONMENT-$RESOURCE-service"
TASK_DEF_NAME="$ENVIRONMENT-$RESOURCE-task-def"
ECR_REPO="$RESOURCE-$ENVIRONMENT"

# Construct full image URI
ECR_URL="$ECR_REGISTRY/$ECR_REPO"
FULL_IMAGE_URI="$ECR_URL:$IMAGE_TAG"
echo "Using image URI: $FULL_IMAGE_URI"

# Fetch the existing definition of the task and create a new revision with the updated URI
TASK_DEFINITION=$(aws ecs describe-task-definition --task-definition $TASK_DEF_NAME --region $REGION --query 'taskDefinition')
NEW_TASK_DEF=$(echo $TASK_DEFINITION | \
  jq --arg IMAGE_URI "$FULL_IMAGE_URI" '.containerDefinitions[0].image = $IMAGE_URI' | \
  jq 'del(.taskDefinitionArn, .revision, .status, .requiresAttributes, .compatibilities, .registeredAt, .registeredBy)' | \
  jq -c)

# Register the new task definition
NEW_TASK_INFO=$(aws ecs register-task-definition --cli-input-json "$NEW_TASK_DEF" --region $REGION)
NEW_REVISION=$(echo $NEW_TASK_INFO | jq -r '.taskDefinition.revision')
echo "Created new task revision: $NEW_REVISION"

# Update the ECS cluster to the new revision
aws ecs update-service --cluster $CLUSTER_NAME --service $SERVICE_NAME --task-definition $TASK_DEF_NAME:$NEW_REVISION --region $REGION >/dev/null 2>&1
echo "ECS cluster updated to new revision"