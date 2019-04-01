ifneq (${HAB_SUP_TEST_NETWORK_SIZE},)
	peers := ${HAB_SUP_TEST_NETWORK_SIZE}
else
	peers := 5
endif

start: CTL_SECRET
	@docker build -t hab-sup-network .
	@docker-compose up --scale peers=${peers}

stop:
	@docker-compose down
	@rm -f CTL_SECRET

console:
	@docker exec -it $$(docker ps -qf name=bastion) /bin/bash

CTL_SECRET:
	@hab sup secret generate > CTL_SECRET
